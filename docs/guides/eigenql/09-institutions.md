# 8. Institutions in EigenQL

Institutions are domain-specific reasoning systems registered with the kernel as ontology declarations. Under D14 ([Institution Realisation](../../design/d14-institution-realisation.md), supersedes D10), every concept an institution exposes — the institution itself, its export/import boundary formats, its query classes, its comorphisms — is a typed `Resource` committed to the layer chain. The kernel's institution registry is a derived index built by scanning that chain (see [§9 of D14](../../design/d14-institution-realisation.md)).

This chapter focuses on what EigenQL sees of that surface: how qualified-name function calls dispatch to a `Decidable` `QueryClass`, how the resulting `Verdict` becomes a Boolean via a postfix predicate, how comorphisms enter only through `FIBER` parameter coercion, and how the classification table differs from the legacy D10 form.

## 9.1. What lives in the chain

Five resource shapes carry an institution's declared surface (D14 §4):

| Shape | What it declares |
|---|---|
| `Institution` | Identity (`institution_iri`, `name`) and `runtime` kind (`wasm`, `external`, `in_process`). |
| `ExportFormat` | A typed *outbound* view: extracts a EigenTT-typed payload (`payload_type`) from a source resource of a given `from_class`, via a procedure handled by the institution. |
| `ImportFormat` | The dual: constructs a target-class resource from a typed payload, via a procedure handled by the institution. |
| `QueryClass` | A typed function in the institution's fibre. Carries `query_class` (input class), `result_class`, a `dispatch_role` set (`OnDemand` ∪ `AutoOnLoad` ∪ `Decidable`), a `query_handler` procedure IRI, and an `institution_ref`. |
| `Comorphism` | The triadic translation `(s, m, t)`: `export_format` + `transformation` (a EigenTT Component) + `import_format`, plus an `exact: bool` Satisfaction-Condition annotation. |

EigenQL's parser, type checker, and evaluator consult two derived structures built from those declarations:

- [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) — by-IRI lookups for the five shapes above. Built by `InstitutionIndex::from_layer` on the active layer chain. EigenQL's compile-time classifier (§9.5) uses it to decide what kind of call a qualified-name IRI is.
- [`InstitutionRuntime`](../../../kernel/src/institution/runtime.rs) — `BTreeMap<Iri, Box<dyn Institution>>` keyed by the institution's IRI. Used to dispatch `extract_typed` / `reify` / `query` calls (D14 §8) when an EigenQL evaluator step needs runtime-side reasoning.

The runtime is rebuilt on every commit by the kernel's [`build_wasm_institution_runtime`](../../../kernel/src/capability/registration.rs) — every `Institution` resource on the chain whose `runtime` is `wasm` produces a `WasmInstitution` registered against its IRI. **Substrate-hosted institutions** (`runtime: external`, where the worker lives in a sibling container managed by the orchestrator's substrate addon — Julia v1, Python and others tracked in [issue #41](https://github.com/eigenius/eigenius/issues/41)) are registered through a parallel mechanism that resolves `Institution.requires_environment` to a `RuntimeEnvironment` resource and dispatches via the substrate's `LanguageRuntime` trait. From EigenQL's perspective there's no difference: both kinds answer `Institution::query` against the same trait surface, with the same `extract_typed` / `reify` four-step pipeline. See [platform §10](../platform/10-wasm-institutions.md) for the WASM-hosted path and [platform §11](../platform/11-runtime-substrate.md) for the substrate path. **In-process** runtimes (`runtime: in_process`, kernel-embedded Rust) are caller-registered.

## 9.2. The classification at parse time

When the EigenQL parser sees a `qualified_call` (`ns:local(args)`), the type checker resolves it to a full IRI and looks it up in the `InstitutionIndex`. Three institution-level kinds are recognised under D14 (D14 §10.5):

| Index entry | EigenQL emits | Runtime call |
|---|---|---|
| `QueryClass` whose `dispatch_role` includes `Decidable` | `Exp::NativeDecide(Constraint::Institution { iri, args }, Unit)` (returns a `Verdict`) | `Institution::query(query_handler, synthetic_input, ctx)` |
| `QueryClass` whose `dispatch_role` includes `OnDemand` | Reachable from `FIBER` clauses only — never from expression position | `Institution::query` (during `FIBER` evaluation) |
| `Comorphism` | Reachable from `FIBER` parameter coercion only — never from expression position | `extract_typed → transformation → reify` four-step pipeline (D14 §10.3) |

Anything else falls through. Built-in functions (`LENGTH`, `CONTAINS`, …) are matched first as a closed set; aggregate functions are matched in `RETURN` only. A qualified name that doesn't resolve to any of these is reported as `unknown function: ns:local`.

The two notable retirements compared with D10:

- **No `DecidePredicate` returning Boolean.** A Decidable `QueryClass` returns a `Verdict`, not a Boolean. Use a postfix predicate (§9.4) to project it.
- **No comorphism-as-expression.** D10 allowed `inst:translate(source)` in `RETURN`; D14 does not. Comorphism dispatch surfaces *only* in `FIBER` parameter coercion (§9.6).

## 9.3. Decidable QueryClass invocation

Surface syntax:

```
inst:predicate(arg1, arg2, ...)
```

Result type: `Verdict` — an inductive value with three constructors: `Holds`, `Fails`, `Undecidable` (D14 §7.1).

Evaluation, from [`eval_ctx`'s `NativeDecide` arm](../../../kernel/src/nbe/eval.rs):

1. Look up the constraint IRI in the `InstitutionIndex` as a `Decidable` `QueryClass`. The index returns the declaring institution, the `query_handler` procedure, and the input-class IRI.
2. Marshal the positional `args` into a synthetic input resource. The kernel attaches the args as the special property `urn:eigenius:institution:decide_args` so the institution's handler can read them positionally — alongside any property-shaped marshalling driven by the input class.
3. Resolve the institution in the `InstitutionRuntime` by `institution_ref`. Call `Institution::query(query_handler, synthetic_input, ctx)`.
4. Parse the returned resource as a `Verdict` — its `urn:eigenius:core:ctor_name` property selects `Holds | Fails | Undecidable`.

A bare Verdict-typed expression in Boolean position is a type error (`bare_verdict_in_boolean_position`, §5.9 of D2). The conversion to Boolean is always explicit.

## 9.4. Postfix Verdict predicates: `HOLDS`, `FAILS`, `UNDECIDABLE`

A postfix predicate projects a `Verdict` to a Boolean:

```eigenql
WHERE qc:check(?x) HOLDS                -- Decidable QC: include if Holds
WHERE NOT qc:check(?x) HOLDS            -- "Fails or Undecidable"
WHERE qc:a(?x) HOLDS AND qc:b(?y) FAILS -- Verdicts in conjunction
RETURN [] { ok: qc:check(?x) HOLDS }    -- Boolean projection in RETURN
```

The predicate binds tighter than `NOT` and looser than primary-expression construction. It is non-associative: `?v HOLDS FAILS` is a syntax error.

The operand must be a `Verdict`. The static type checker accepts only two source forms (D14 §5.9):

1. A `qualified_call` that resolves to a `Decidable` `QueryClass`.
2. A variable bound by a `FIBER` clause whose `result_class` is `urn:eigenius:institution:Verdict`.

Other expressions — string columns, arithmetic, FIBER bindings whose result class isn't Verdict — are rejected as `verdict_predicate_non_verdict_operand`. This deliberate narrowness makes the surface language honest about where Verdicts come from; future revisions could generalise.

## 9.5. FIBER clauses against an `OnDemand` QueryClass

```eigenql
USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay

FIBER assay:validate_prediction {
    candidate: ?p
} AS ?check
WHERE ?check HOLDS
```

The clause names a `QueryClass` whose `dispatch_role` set includes `OnDemand`. The evaluator builds an input resource whose `is_a` is the QueryClass's `query_class`, sets the param bindings as properties, looks up the QueryClass's `institution_ref` in the `InstitutionRuntime`, and calls `Institution::query(query_handler, input, ctx)`. The returned resource is bound to `?check` and visible to subsequent clauses (§7).

The `FIBER` form is the only path under D14 that returns a structured response resource — for example a multi-property answer to a domain query. Decidable QueryClasses (which return Verdict directly) and Comorphism coercions (which surface only inside FIBER param values) cover the other two roles.

## 9.6. Comorphism coercion in `FIBER` params

```eigenql
USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay

MATCH DockingResult(?d) { delta_g: ?dg }
FIBER assay:validate_prediction {
    candidate: dock_to_assay(?d)
} AS ?check
WHERE ?check HOLDS
```

`dock_to_assay(?d)` is a *comorphism coercion*: a parameter-position invocation of a declared `Comorphism`. The parser only accepts a `qualified_name(expression)` shape in this slot (D2 §3.5). At evaluation the kernel runs the four-step pipeline (D14 §10.3):

1. Resolve the `Comorphism` resource — read `export_format`, `transformation`, `import_format`.
2. Look up the source institution from `export_format.institution_ref`. Call `Institution::extract_typed(export_format.procedure, ?d, ctx)` → typed payload of the export's `payload_type`.
3. Apply the `transformation` Component via the EigenTT evaluator → typed payload of the import's `payload_type`.
4. Look up the target institution from `import_format.institution_ref`. Call `Institution::reify(import_format.procedure, payload, ctx)` → the target-class resource.

The coerced resource is set on the input as the property whose short name is `candidate`. The type checker enforces that the comorphism's import-side `to_class` matches (or is a subclass of) the QueryClass's required class type for that property; failures surface as `comorphism_coercion_class_mismatch`.

**v1 restriction** (D2 §3.5): the comorphism's `transformation` Component must have `capability_level` `Pure` or `Read`. A Component requiring `IO` is rejected at parse time with `comorphism_io_not_supported_in_v1`. This lets the EigenQL evaluator run the coercion inline without standing up an IO backing for the FIBER path.

Comorphisms are not first-class in expression position inside an EigenQL query — there is no `RETURN [...] { x: cap:translate(?d) }`. EigenQL surfaces comorphisms only as FIBER param coercions, and (by way of `INTO`, see [§8.6](08-fiber-clauses.md#76-into--pinning-the-response-iri)) lets the caller pin the reified output at a named chain IRI. ESL programs *do* invoke comorphisms in expression position; see [ESL §10.5](../esl/09-institutions.md#95-invoking-comorphisms-from-esl-programs).

**Chain reinsertion (D14 §10.3).** Whichever surface invokes the comorphism, the reified output commits to the chain. With `FIBER ... AS ?var` (no `INTO`), the response lives in the transient overlay only — the v0 behaviour, kept for queries that don't want side effects. With `FIBER ... AS ?var INTO "<iri>"`, the response commits to the regular chain at the caller-named IRI. With ESL `comorphisms:foo(input)` from a program body, the response commits at a deterministic content-hash IRI of the form `urn:eigenius:comorphism-output:<comorphism-tail>:<hex16>` (kernel-minted). All three paths share the same `commit_with_validation` machinery — AutoOnLoad gates bound to the produced class fire on commit; the audit closure (Verdict + RuntimeInvocation) is the standard one.

## 9.7. A complete worked example

Drawn from [`kernel/tests/d14_dock_assay_demo.rs`](../../../kernel/tests/d14_dock_assay_demo.rs) (the M8 in-process demo): two institutions — `dock` (extracts ΔG from a `DockingResult`) and `assay` (reifies an `AssayPrediction` from an IC₅₀, plus three QueryClasses) — and one Comorphism (`dock_to_assay`, the Arrhenius approximation `IC₅₀ ≈ exp(-ΔG/RT)`).

The chain carries `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, and `Comorphism` resources from [`ontologies/examples/d14-dock-assay/dock-assay.json`](../../../ontologies/examples/d14-dock-assay/dock-assay.json). With an `InstitutionIndex` and an `InstitutionRuntime` populated from those declarations, this query returns rows whose docking results survive the assay's validity check after Arrhenius coercion:

```eigenql
USING "urn:eigenius:demo:d14:DockingResult"
USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay

MATCH DockingResult(?d) {
    "urn:eigenius:demo:d14:delta_g": ?dg
}
FIBER assay:validate_prediction {
    candidate: dock_to_assay(?d)
} AS ?check
WHERE ?check HOLDS
RETURN [] {
    docking: ?d,
    delta_g: ?dg
}
```

What runs:

- The `MATCH` produces one binding per `DockingResult` in scope.
- For each binding, `FIBER` invokes `dock_to_assay`'s four-step pipeline: dock's `extract_typed` returns ΔG as a `Float`; the `cm_arrhenius` Component computes `exp(-Δg/RT) · 1e9`; assay's `reify` constructs an `AssayPrediction` with that `ic50`. The synthesized `validate_prediction` input resource carries the prediction as `candidate`, and assay's `query` handler returns a `Verdict`.
- `WHERE ?check HOLDS` keeps only the `Holds` verdicts; rows where the IC₅₀ came out non-positive (`Fails`) or the input was missing the required field (`Undecidable`) are dropped.

To execute this in code, the calling test wires the kernel like so:

```rust
let layer = layer_with_demo_ontology();                    // base + child layers
let (idx, _) = InstitutionIndex::from_layer(&layer);
let runtime: InstitutionRuntime = build_demo_runtime();
let components: ComponentRegistry = build_demo_components();
let exec_ctx = ExecutionContext::new(Arc::new(layer.clone()), "query", ExecutionMode::ReadOnly);

let fiber_runtime = FiberRuntime {
    index: Some(&idx),
    runtime: Some(&runtime),
    components: Some(&components),
    ctx: Some(&exec_ctx),
    overlay: None,
};

let results = eigenius_kernel::query::execute_with(query_text, &layer, fiber_runtime)?;
```

When the kernel hosts the institutions as WASM (D14 §13), the wiring is even shorter — the `InstitutionRuntime` is auto-built by `build_wasm_institution_runtime` from the `runtime: wasm` declarations on the chain (see [platform §10](../platform/10-wasm-institutions.md) and [`kernel/tests/d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs)).

## 9.8. Comparison with FIBER

| Dimension | Decidable QueryClass | FIBER (OnDemand QueryClass) | FIBER param comorphism |
|---|---|---|---|
| Position | `WHERE` / `RETURN` expression (`HOLDS`/`FAILS`/`UNDECIDABLE` projection) | Top-level FIBER clause | Inside a FIBER param value |
| Returns | `Verdict` (3-constructor inductive) | Caller-declared `result_class` | A target-class resource (the FIBER's input slot) |
| Param shape | Positional args | Property-shaped param block (subject to type checks) | Single `qualified_name(expression)` |
| Type-check | Decidable lookup; arity validated by handler | `requires` from input class enforced; comorphism coercions checked against expected class | Comorphism `to_class` ⇒ FIBER input class compatibility |

Choose Decidable when the institution's answer is simply `Holds | Fails | Undecidable` over a finite arg list. Choose FIBER when you need a structured response. Use comorphism coercion inside FIBER when the FIBER's input has to be in a different institution's vocabulary than what you have in your binding.

## 9.9. The classification table

Quick reference — the same classifier ESL uses (see [ESL chapter 10](../esl/09-institutions.md)):

| Index lookup | ESL emits | EigenQL emits | Runtime call |
|---|---|---|---|
| `Decidable` QueryClass | `Exp::NativeDecide(Constraint::Institution { … }, Unit)` (Verdict-typed) | Same as ESL — and the postfix predicate projects to Boolean | `Institution::query(query_handler, synthetic_input, ctx)` |
| `OnDemand` QueryClass | not exposed in ESL expression position | `FIBER` clause | `Institution::query(query_handler, input, ctx)` |
| `Comorphism` | qualified-name function call in program body — `comorphisms:foo(input)` — lowers to `Exp::InstitutionInvoke` (D14 §10.3) | FIBER param value coercion (overlay-only); or `FIBER ... INTO "<iri>"` for chain reinsertion at a caller-named IRI | `extract_typed → transformation Component → reify` four-step pipeline; reify output commits to the chain |
| Class / property / built-in / aggregate | various | various | no institution call |

Both surface languages share `InstitutionIndex` so the same IRI dispatches identically.

## 9.10. When dispatch isn't available

If the kernel's `InstitutionIndex` is empty (no chain scan has happened, or no institutions are declared) and your query references a qualified-name IRI:

- A bare `qc:check(?x)` falls through to "unknown function".
- A `FIBER alias:Q { … }` errors with `undeclared_institution_alias` if no `USING INSTITUTION` is in scope, or `unknown_query_class` if the declared QC is missing from the index.
- A FIBER param comorphism coercion errors with `unknown_comorphism` if the IRI isn't in the index.

If the IRI *is* indexed but the runtime can't reach the institution (no `WasmInstitution` registered, no in-process `Institution` registered for an IRI of `runtime: in_process`), evaluation surfaces `institution_not_in_runtime: <iri>` at the dispatch site.

To debug:

1. Confirm the declarations are present in the chain you query against — `eigenius inspect <iri>` for each declared resource.
2. Confirm `InstitutionIndex::from_layer` returned no errors — the kernel logs them at startup with `operation: institution_register`.
3. For WASM-runtime institutions, confirm `build_wasm_institution_runtime`'s report is clean (the `EigeniusService` logs each registered institution at info level).

---

Next: **[10. Stratification →](10-stratification.md)**
