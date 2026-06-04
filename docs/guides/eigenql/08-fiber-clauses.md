# 7. FIBER clauses

`FIBER` clauses extend a query with reasoning delegated to a registered institution. The specification is in [D2 §3.5 + §5.8 + §7.12](../../design/d2-eigenql-specification.md); the underlying institution model is [D14](../../design/d14-institution-realisation.md). The implementation is [`apply_fiber_clause`](../../../kernel/src/query/evaluate.rs) in `evaluate.rs`.

The pattern: you declare an institution alias, name an `OnDemand` `QueryClass`, supply property-shaped param bindings (with optional comorphism coercion in param values), and bind the response resource to a variable that subsequent clauses can match against.

## 8.1. Anatomy

```
UsingInstitution    ::= 'USING' 'INSTITUTION' StringLit 'AS' ident

FiberClause         ::= 'FIBER' institution_ref ':' QueryClass
                        '{' ParamBinding (',' ParamBinding)* '}'
                        'AS' Variable

ParamBinding        ::= Name ':' ParamValue
ParamValue          ::= Expression | ComorphismCoercion
ComorphismCoercion  ::= QualifiedName '(' Expression ')'
```

Where `institution_ref` is either an alias (`ShortName`) or an inline full IRI (`FullIri`). The QueryClass referenced by the clause must declare `OnDemand` in its `dispatch_role` set (D14 §7.2).

```eigenql
USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay

FIBER assay:validate_prediction {
    candidate: ?p
} AS ?check
```

**`USING INSTITUTION`** introduces the alias at the top of a `MatchPart`. Aliases are scoped to their `MatchPart` and must be unique within that scope; re-declaring is a type error (`duplicate_using_institution_alias`).

**`FIBER`** performs the dispatch. Its three parts:

1. **`institution:query_class`** — who to ask and which `OnDemand` `QueryClass`.
2. **`{ param: value, ... }`** — the input resource's property bindings. A `value` is either a regular expression, or a comorphism coercion (`comorphism_iri(source)`) that runs the four-step extract → transform → reify pipeline inline. See §8.5 below.
3. **`AS ?var`** — where to put the response IRI in the current binding.

## 8.2. Evaluation walk-through

For each binding in the current binding set, `apply_fiber_clause`:

1. **Resolves the institution and QueryClass.** Alias lookup against the `MatchPart`'s `USING INSTITUTION` aliases (or inline IRI). The QueryClass IRI is resolved through the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs); the cited QueryClass's `institution_ref` must equal the aliased / inline institution (`fiber_institution_mismatch` if not), and `OnDemand` must be among its `dispatch_role` values (`fiber_query_class_not_on_demand` if not).
2. **Builds an input resource.** Starts with `is_a: [QueryClass.query_class]` (the input class declared by the QueryClass). For each `param`:
   - The param name is resolved to a property IRI by walking the input class's `requires` ∪ `recommends` looking for `short_name` matches; full-IRI param names pass through.
   - If the value is a regular expression, it's evaluated with the current binding.
   - If the value is a comorphism coercion, the kernel runs the four-step pipeline (§8.5) and uses the reified target-class resource as the property's value.
3. **Dispatches.** Looks up the QueryClass's `institution_ref` in the [`InstitutionRuntime`](../../../kernel/src/institution/runtime.rs) and calls `Institution::query(query_handler, input_resource, ctx)` (D14 §8). The institution returns a result resource.
4. **Stamps the response.** The response gets a deterministic IRI via `fp.fiber_response_iri(clause_idx, binding_idx)` — stable per (query text, clause, binding). Subsequent runs against the same inputs produce the same overlay resource identity.
5. **Attaches to the overlay.** The stamped resource is pushed into the `FiberOverlay` — visible to all subsequent pattern matching in this query only.
6. **Extends the binding.** `?check` is bound to the response IRI as a `Value::String`.

Step 6 is why subsequent clauses can reference `?check` just like any resource:

```eigenql
FIBER assay:validate_prediction { candidate: ?p } AS ?check
WHERE ?check HOLDS
```

The `WHERE` clause uses a postfix Verdict predicate (§9.4) to project the bound Verdict response to a Boolean — see [chapter 9](09-institutions.md) for the full Verdict surface.

## 8.3. Requirements on the runtime

`FIBER` clauses require the [`FiberRuntime`](../../../kernel/src/query/evaluate.rs) to carry the D14 dispatch handles:

```rust
pub struct FiberRuntime<'a> {
    pub index: Option<&'a InstitutionIndex>,
    pub runtime: Option<&'a InstitutionRuntime>,
    pub components: Option<&'a ComponentRegistry>,
    pub overlay: Option<&'a [(Iri, Resource)]>,
    pub ctx: Option<&'a ExecutionContext>,
}
```

- `index` and `runtime` are required for any FIBER clause: the index resolves the QueryClass and the runtime answers the dispatch.
- `components` is required when any FIBER param value uses comorphism coercion (§8.5) — the four-step pipeline applies the comorphism's transformation Component, which lives in the component registry. v1 restricts the cited transformation to `Pure` or `Read` capability levels.
- `ctx` is required for any FIBER clause; the institution's handler may need read access to the layer chain through the execution context.
- `overlay` is populated automatically as preceding FIBER clauses add their responses; you don't pass it in.

Queries executed via the convenience `execute(program_str, layer)` (no runtime) **cannot use `FIBER`** — it constructs a `FiberRuntime::default()` with all fields `None` and dispatch errors at the first FIBER clause. Use `execute_with(program_str, layer, FiberRuntime { … })` when your query needs FIBER dispatch.

## 8.4. The transient overlay

The overlay model (D2 §7.12) isolates FIBER responses from the persistent layer:

- Responses live in `FiberOverlay` for the duration of a single query evaluation.
- Pattern matching in subsequent clauses scans the overlay in addition to the layer (and derived relations).
- The overlay is discarded when `evaluate()` returns. Nothing is committed to the layer.

This lets queries use institution reasoning without side effects. Two successive runs of the same query produce identical results only if the institution's `query()` is deterministic — the overlay IRIs themselves are stable because `fp.fiber_response_iri` is deterministic, but the institution is free to return different responses.

## 8.5. Comorphism coercion in param values

A FIBER param value can be a *comorphism coercion* — `comorphism_iri(source_expression)` — which runs the four-step pipeline (D14 §10.3) inline and uses the reified target-class resource as the property's value:

```eigenql
USING INSTITUTION "urn:eigenius:demo:d14:assay" AS assay

MATCH DockingResult(?d) { delta_g: ?dg }
FIBER assay:validate_prediction {
    candidate: dock_to_assay(?d)
} AS ?check
WHERE ?check HOLDS
```

Here `dock_to_assay` is a `Comorphism` resource (D14 §4.5). At evaluation:

1. The kernel resolves the comorphism in the `InstitutionIndex` (read `export_format`, `transformation`, `import_format`).
2. **Extract.** Calls the source institution's `Institution::extract_typed(export_format.procedure, ?d, ctx)` → typed payload of the export's `payload_type`.
3. **Transform.** Applies the transformation Component to the payload via the kernel's component evaluator → typed payload of the import's `payload_type`. The Component must be registered in the `FiberRuntime.components` and its `capability_level` must be `Pure` or `Read` (see §8.3).
4. **Reify.** Calls the target institution's `Institution::reify(import_format.procedure, payload, ctx)` → the target-class resource.

The reified resource is set as the property's value on the FIBER's input resource.

Under D14 §10.3, the reified resource also commits to the chain at a deterministic content-hash IRI of the form `urn:eigenius:comorphism-output:<comorphism-tail>:<hex16>` (SHA-256 over canonical Eigon-CBOR with `@id` cleared). Re-running the same input dedupes to the same IRI. AutoOnLoad gates bound to the produced class fire on the resulting commit exactly as if the resource had been authored by hand.

The type checker validates that the cited comorphism's import-side `to_class` matches (or is a subclass of) the QueryClass's expected class type for that property (`comorphism_coercion_class_mismatch` if not), and that the transformation Component is `Pure` or `Read` (`comorphism_io_not_supported_in_v1` if `IO` — D2 v1 restriction).

Comorphisms are *not* available in expression position elsewhere in EigenQL — only inside FIBER param values, and (in ESL) as qualified-name function calls in program bodies. See [chapter 9](09-institutions.md) for the full classification table and [ESL §10.5](../esl/09-institutions.md#95-invoking-comorphisms-from-esl-programs) for the ESL surface.

## 8.6. `INTO` — pinning the response IRI

A `FIBER` clause can name an explicit IRI for its response via the `INTO` keyword, written after `AS ?var`:

```eigenql
USING INSTITUTION "urn:eigenius:institutions:symbolics" AS symb

FIBER symb:"urn:eigenius:symbolics:query_classes:qc_symb_to_jump" {
    "urn:eigenius:symbolics:objective":               "urn:…:sse_expr",
    "urn:eigenius:symbolics:variable_names":          ["Ki"],
    "urn:eigenius:symbolics:framing_variable_bounds": ["urn:…:ki_bound"],
    "urn:eigenius:symbolics:sense":                   "Min"
} AS ?problem INTO "urn:eigenius:notebook:kinase_demo:fiber_problem"
RETURN [] { iri: ?problem }
```

The dispatch is identical to a plain `FIBER ... AS ?var` clause; the difference is **chain reinsertion**: the response resource commits to the regular chain at the named IRI, rather than living in a transient overlay. Both surfaces share the same `commit_with_validation` machinery — AutoOnLoad gates bound to the response's class fire on commit, the response is reachable by every subsequent query, and the audit trail (Verdict + RuntimeInvocation) is the standard one.

The `INTO` IRI is mandatory if you want chain reinsertion; without it, the response stays overlay-only (the v0 behaviour). Pinning at a caller-named IRI is the contrast with the comorphism program-invoke form (§8.5 above), where the kernel mints a *content-hash* IRI for you.

`INTO` is a hard keyword (D2 v2 §3.5). The full grammar:

```
FiberClause ::= 'FIBER' institution_ref ':' QueryClass
                '{' params '}'
                'AS' '?'<ident>
                ('INTO' StringLiteral)?
```

The string after `INTO` must be a syntactically-valid IRI; the type checker validates it (`fiber_into_iri_invalid` if not).

## 8.7. FIBER vs. Decidable vs. comorphism coercion

Three ways an EigenQL query can involve an institution. They have different shapes and semantics:

| Mechanism | Syntax | Returns | When to use |
|---|---|---|---|
| Decidable QueryClass | `inst:predicate(args) HOLDS` (or `FAILS`/`UNDECIDABLE`) in `WHERE`/`RETURN` | `Verdict` projected to Boolean by the postfix predicate | Yes/no filtering with explicit handling of Undecidable |
| `FIBER` clause | `FIBER inst:QueryClass { params } AS ?var` | Binds an institution-returned response resource (whose class is the QueryClass's `result_class`) | Multi-property responses; the bound variable participates in later pattern matching |
| Comorphism coercion | `param: comorphism_iri(source)` inside a FIBER param block | A target-class resource set on the FIBER's input | Crossing an institution boundary into the FIBER's input vocabulary |

A query can mix all three. FIBER is the most powerful shape because the bound variable participates in later pattern matching; Decidable is the simplest call that produces a single Verdict; comorphism coercion is the controlled boundary-crossing form.

## 8.8. Type-checking rules

The type checker ([kernel/src/query/type_check.rs](../../../kernel/src/query/type_check.rs)) validates:

- **`undeclared_institution_alias`** — the alias in `FIBER alias:Q { … }` wasn't declared with `USING INSTITUTION`.
- **`duplicate_using_institution_alias`** — two `USING INSTITUTION` clauses in the same `MatchPart` use the same alias.
- **`fiber_query_class_not_class`** — the query class IRI doesn't resolve, or resolves to something that isn't a `Class`.
- **`fiber_query_class_not_on_demand`** — the QueryClass is declared but its `dispatch_role` set doesn't include `OnDemand`.
- **`fiber_institution_mismatch`** — the QueryClass's `institution_ref` doesn't match the alias / inline IRI in the FIBER clause.
- **`fiber_param_short_name_unresolved`** — a param short name isn't a property that `requires` or `recommends` declares on the query class.
- **`fiber_missing_required_param`** — a `requires` property of the query class wasn't supplied in the param list.
- **`comorphism_coercion_class_mismatch`** — a comorphism coercion's import-side `to_class` doesn't satisfy the FIBER input class's expected type for that property.
- **`comorphism_io_not_supported_in_v1`** — a comorphism's transformation Component requires `IO` capability.
- **`unknown_comorphism`** — a comorphism coercion references an IRI that isn't a registered `Comorphism`.

The check runs before evaluation, so malformed FIBER clauses fail fast at compile time.

## 8.9. Determinism and the response IRI

Response IRIs are generated by [`QueryFingerprint::fiber_response_iri(clause_idx, binding_idx)`](../../../kernel/src/query/document.rs) — a deterministic function of the query text (hashed to a fingerprint), the clause index within the query, and the binding index within the clause's iteration.

This matters because:

1. **Reproducibility**: re-running the same query over the same layer produces the same overlay IRIs (institution response content may vary if the institution is non-deterministic, but IRIs are stable).
2. **Caching**: higher layers that cache query results can key on both the query text and the overlay IRI scheme.
3. **Debugging**: a response IRI contains the query hash, so error messages reference traceable identities.

The response IRI format is `urn:eigenius:query:gen:<8-hex>:fiber:<clause>:<binding>`.

## 8.10. Example: comorphism coercion + Verdict postfix

Drawn from the M8 worked example ([`kernel/tests/d14_dock_assay_demo.rs`](../../../kernel/tests/d14_dock_assay_demo.rs)) — chains a comorphism coercion through a FIBER `OnDemand` QueryClass and projects the resulting Verdict to a Boolean filter:

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

For each docking result the kernel:

1. Runs `dock_to_assay`'s four-step pipeline — extract ΔG via the dock institution, apply the Arrhenius transformation Component, reify an `AssayPrediction` via the assay institution.
2. Synthesises the FIBER input as `validate_prediction`'s declared input class with `candidate` set to that prediction.
3. Calls `assay`'s `query` handler for `validate_prediction` — returns a `Verdict` resource.
4. Projects the Verdict via `?check HOLDS` — keeps rows where the assay institution accepts the predicted IC₅₀ as physically plausible.

Behaviour notes:

- The Verdict bound to `?check` lives in the overlay — it's referenceable by IRI as a regular resource, not just through the postfix predicate.
- If `?d` yields N bindings, N four-step pipelines + N `validate_prediction` dispatches run sequentially. FIBER is not batched yet.

---

Next: **[9. Institutions →](09-institutions.md)**
