# 8. Institutions in EigenQL

Institutions are domain-specific reasoning modules registered with the kernel. They implement the [`FiberReasoner`](../../../kernel/src/institution/mod.rs) trait and declare three kinds of capability that EigenQL can invoke:

1. **Query types** — structured resource-in / resource-out reasoning, invoked via `FIBER` clauses ([chapter 7](07-fiber-clauses.md))
2. **Decide predicates** — boolean predicates, invoked as `cap:predicate(args)` in expression position (Phase 11c + 11e.2)
3. **Comorphisms** — cross-institution resource translations, invoked as `cap:translate(source)` in expression position (Phase 11d + 11e.2)

This chapter focuses on capability classification — how EigenQL decides which path a function-call IRI takes.

## 8.1. What institutions declare

An institution's [`FiberDeclaration`](../../../kernel/src/institution/mod.rs) has five capability fields:

```rust
pub struct FiberDeclaration {
    pub institution_iri: Iri,
    pub name: String,
    pub morphism_types: Vec<Resource>,
    pub query_types: Vec<Resource>,
    pub structural_properties: Vec<Resource>,
    pub comorphism_types: Vec<Resource>,      // Phase 11d
    pub decide_procedures: Vec<Iri>,          // Phase 11e.1
}
```

The two that matter for EigenQL's institution-dispatched function calls are `comorphism_types` (IRIs of `Comorphism`-class resources this institution declares) and `decide_procedures` (IRIs of decide predicates this institution answers).

When the institution registers via [`InstitutionRegistry::register`](../../../kernel/src/institution/mod.rs), the registry builds two dispatch tables:

- `comorphism_dispatch: BTreeMap<Iri, Iri>` — comorphism IRI → declaring institution IRI
- `decide_dispatch: BTreeMap<Iri, Iri>` — procedure IRI → declaring institution IRI

`InstitutionRegistry::classify(iri)` looks up the IRI in both tables and returns:

```rust
pub enum InstitutionCapability {
    DecidePredicate,
    Comorphism,
}
```

or `None` if the IRI isn't registered as either. This is the single point of classification shared by ESL's compile-time dispatcher (Phase 11e.1) and EigenQL's evaluate-time dispatcher (Phase 11e.2).

## 8.2. Invoking a decide predicate

Surface syntax:

```
inst:predicate(arg1, arg2, ...)
```

Evaluation, from [`eval_expression`](../../../kernel/src/query/evaluate.rs)'s `FunctionCall` arm:

1. If `name` contains `:`, try parsing as an IRI.
2. If the parse succeeds and `registry.classify(&iri)` returns `Some(DecidePredicate)`, dispatch via [`dispatch_institution_call`](../../../kernel/src/query/evaluate.rs):
   - Look up the declaring institution via `registry.institution_for_decide(&iri)`.
   - Call `reasoner.decide(&iri, &arg_values, &exec_ctx)`.
3. Map the three-valued result to a boolean: `Holds → true`, `Fails → false`, `Undecidable → false`.

Used in `WHERE`:

```eigenql
WHERE docking:within_tolerance(?delta, 2.0)
```

Used in `RETURN` (produces a boolean column):

```eigenql
RETURN [] {
    delta: ?delta,
    is_valid: docking:within_tolerance(?delta, 2.0)
}
```

Used in `ORDER BY` or `GROUP BY`:

```eigenql
GROUP BY docking:category(?d)   -- if the institution had a categorical decide
```

The boolean mapping `Undecidable → false` is a **WHERE-semantics-first** choice. If you need three-valued semantics in downstream logic, you'll want to wrap the result in a way that preserves it — or switch to a FIBER clause that returns a richer response resource.

## 8.3. Invoking a comorphism

Surface syntax:

```
inst:comorphism(source)
```

Exactly one argument — the source resource. More arguments produce an evaluation error.

Evaluation:

1. If the IRI classifies as `Some(Comorphism)`, look up the institution via `registry.institution_for_comorphism(&iri)`.
2. Convert the argument to a `Resource`:
   - If the arg evaluated to `Value::Embedded(r)`, use the embedded resource.
   - Otherwise, wrap the scalar in an embedded resource with a `urn:eigenius:core:value` property.
3. Call `reasoner.translate(&iri, &source_resource, &exec_ctx)`.
4. Wrap the returned resource in `Value::Embedded(Box::new(..))`.

Used in `RETURN`:

```eigenql
RETURN [] {
    compound: ?c,
    predicted_ic50: docking:dock_to_assay(?docking_result)
}
```

The `predicted_ic50` column contains an embedded resource per row.

Used as a nested expression:

```eigenql
WHERE LENGTH(docking:dock_to_assay(?d)) > 0    -- unusual but legal
```

## 8.4. When an IRI isn't registered

If `name` contains `:` but the IRI doesn't classify (not a decide, not a comorphism), the evaluator falls through to builtin dispatch. [`functions::call_function`](../../../kernel/src/query/functions.rs) doesn't recognize qualified IRIs as builtin names, so you get:

```
unknown function: urn:eigenius:cap:unregistered
```

This is the same error as misspelling a builtin (`LEGNTH(?x)` instead of `LENGTH(?x)`) — the error points at the function name either way.

To debug, check:

1. The institution registered successfully (`InstitutionRegistry::register` returned `Ok`).
2. The IRI is spelled correctly in both the query and `FiberDeclaration.decide_procedures` or `.comorphism_types`.
3. The `FiberRuntime` passed to `execute_with` has `institutions: Some(registry)` — without it, `eval_expression` receives `None` and can't classify.

## 8.5. Comparison with FIBER

FIBER and function-call dispatch differ in four ways:

| Dimension | FIBER clause | Function call |
|---|---|---|
| Position | Top-level clause | Expression |
| Return shape | Bound to a variable; pattern-matchable | Value (bool or embedded resource) |
| Param shape | Query-class-typed resource with property checks | Positional arguments, institution-validated |
| Type-check | Structural (class + required params exist) | Light (classification only) |

Choose FIBER when the institution's response is a multi-property resource that subsequent clauses want to pattern-match. Choose a decide or comorphism call when the result is a single value.

## 8.6. The classification table

For quick reference — the shared classification across ESL and EigenQL:

| IRI resolves to | ESL emits | EigenQL emits | Runtime call |
|---|---|---|---|
| Registered comorphism | `Exp::InstitutionInvoke { iri, source }` | `dispatch_institution_call(Comorphism, ..)` | `reasoner.translate` |
| Registered decide procedure | `Exp::NativeDecide(Constraint::Institution{..}, Unit)` | `dispatch_institution_call(DecidePredicate, ..)` | `reasoner.decide` |
| FIBER query class | n/a | `FIBER` clause → `apply_fiber_clause` | `reasoner.query` |
| Class / primitive / literal | various | various | no institution call |

## 8.7. A complete life-science example

Two institutions — `docking` (decides within-tolerance, defines the `dock_to_assay` comorphism) and `assay` (accepts the translated input). A query that filters by decide and includes the translation:

```eigenql
USING "urn:eigenius:example:Docking"

MATCH Docking(?d) {
    delta_g: ?dg,
    compound: ?cpd
}
WHERE docking:within_tolerance(?dg, 2.0)
RETURN [Prediction] {
    compound: ?cpd,
    delta_g: ?dg,
    assay_ic50: docking:dock_to_assay(?d)
}
```

For each docking result whose ΔG is within the 2.0 tolerance (the docking institution decides via RMSD on its internal pose ensemble), include the compound, raw ΔG, and the translated assay-domain prediction (computed by the docking institution's `dock_to_assay` comorphism — which might under the hood call the `assay` institution, but that's its business; EigenQL just sees a `Resource` come back).

To run this, the caller sets up:

```rust
let mut registry = InstitutionRegistry::new();
registry.register(Box::new(DockingInstitution::new()))?;
registry.register(Box::new(AssayInstitution::new()))?;

let runtime = FiberRuntime {
    institutions: Some(&registry),
    ctx: Some(&exec_ctx),
};

let results = kernel::query::execute_with(query_text, &layer, runtime)?;
```

## 8.8. Choosing which surface to use

Rough rule of thumb:

- Need a yes/no filter in `WHERE`? → **decide predicate**
- Need to produce a structured result field? → **comorphism** if it's a cross-institution translation, **FIBER** if it's a domain computation with multiple output properties
- Need to pattern-match on response structure in later clauses? → **FIBER** (only FIBER puts the response in the overlay for subsequent matching)
- Multiple positional arguments on a structured query? → **FIBER** (decide predicates take positional scalars; comorphisms take exactly one source resource; FIBER takes a param object)

All three paths are first-class — the institution author decides which capabilities to expose based on the domain. Users pick the matching surface at the query site.

---

Next: **[9. Stratification →](09-stratification.md)**
