# D18: Ontology-as-Types Resolution

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 10a)
**Required before:** Phase 10a implementation
**Depends on:** D9 (NbE + type extensions), D13 (durable kernel state)
**Unblocks:** Phase 11 (type-theory extensions), life-science-requirements.md §19

---

## 1. Motivation

Eigenius's type system wants to be the ontology. A resource declared with
`is_a = [urn:eigenius:example:Patent]` should have the type of the
`Patent` class — a dependent record whose fields are whatever the class
declares as required/recommended properties. Property access `r.title`
should type-check against the declared datatype of `Patent.title`.

Today it doesn't. Two leaks in `kernel/src/nbe/check.rs` let untyped
terms sail through:

1. **`find_sigma_field` silently collapses `EigonClass` to `Val::Set`.**
   When the checker is looking up the type of a property on a class-typed
   resource and the enclosing type is `EigonClass(iri)`, the fallback
   branch returns `Val::Set` (the universe of types). The comment in the
   code calls this out: *"could resolve to a Sigma if we had layer
   access. For now, return Set as a fallback for unresolved class
   types."* Any subsequent check against that return either accepts
   nonsense by coincidence (Set is permissive) or rejects a valid
   program with a baffling error.

2. **`check_infer` is missing cases for typed Eigon constructs.** A
   handful of Exp forms — `Construct`, `EigonResource`, `Template`,
   `IdJ`, `Refl`, `NativeDecide`, `DecEq` — only have *checking* rules,
   not inference rules. The moment a user writes `let x = Construct(...)`
   without a type annotation, or passes a `refl(a)` into a position
   that requires inference, the checker says *"cannot infer type of"*
   and gives up. Not a soundness hole but a practical usability
   ceiling: every typed form needs a type annotation from its caller.

This document specifies how to close both gaps. The scope is the
checker (`kernel/src/nbe/check.rs`) plus the ground-type machinery
(`kernel/src/program/ground.rs`) it will invoke. No changes to the
evaluator, the layer system, or the ontology format itself.

### Why now

Life-science §19 names this as step 1 of the domain breadth work:
*"nothing works cleanly until `find_sigma_field` resolves `EigonClass`
to proper dependent records."* Phase 11 (Map/Reduce primitives,
inductive types, decision procedures) assumes that property access on
ontology-typed resources just works. It doesn't yet.

---

## 2. Today's state (honest inventory)

### 2.1 `find_sigma_field`

`kernel/src/nbe/check.rs` around line 510:

```rust
fn find_sigma_field(typ: &Val, field_name: &str) -> Option<Val> {
    match typ {
        Val::Sig(t, g) => { /* walks Sigma chain by local name */ }
        // Also check EigonClass — could resolve to a Sigma if we
        // had layer access. For now, return Set as a fallback for
        // unresolved class types.
        Val::EigonClass(_) => Some(Val::Set),
        _ => None,
    }
}
```

The EigonClass arm is the gap. What *should* happen: when the
property-access target has type `EigonClass(iri)`, resolve `iri` against
the current layer chain, walk the class's declared properties, and
return the declared datatype of the field whose local name matches.

### 2.2 The existing machinery already solves half of it

`kernel/src/program/ground.rs` has `resolve_class_type`:

```rust
pub fn resolve_class_type(class_iri: &Iri, layer: &Layer) -> Result<Val, String>
```

It does exactly what `find_sigma_field` needs: reads the Class resource
from the layer chain, collects required + recommended properties
(including inherited via `subclass_of`), resolves each property's
datatype, and builds a nested `Val::Sig` chain. `find_sigma_field`
currently can't call it because `check_infer` doesn't have a `Layer`
handle — the type-checker entrypoints take only `(rho, gamma, exp)`.

### 2.3 `check_infer` coverage

The match at `check_infer` (around line 200 of check.rs) handles
`Var`, `App`, `Fst`, `Snd`, `PropAccess`, and `Observe`. Everything
else falls through to `"cannot infer type of: {e:?}"`. The missing
cases:

| Exp form | Why it matters |
|----------|---------------|
| `Construct(class_iri, fields)` | `let x = Construct(...)` without annotation is a hard stop today |
| `EigonResource(r)` | Literal resource can't be passed to an inference-mode slot |
| `Template(…)` | Same, for template literals |
| `IdJ(args)` | J-eliminator without explicit motive needs to infer |
| `Refl(a)` | `refl(a)` naked in inference position |
| `NativeDecide(c, v)` | Produces `Refl(v)` or a neutral; type = `Id(A, v, v)` |
| `DecEq(a, x, y)` | Produces `Refl(x)` or a neutral; type = `Id(a, x, y)` |

All have checking rules in `check`, so `let x : T = e` forms work.
The gap is purely inference-mode.

---

## 3. Goals and non-goals

**Goals:**

- `find_sigma_field` walks the layer chain when it encounters
  `EigonClass(iri)`, returning the correctly-typed field instead of
  `Val::Set`.
- Property access on an ontology-typed value type-checks against the
  declared property's datatype, with errors that name the class and
  field rather than leaking Sigma internals.
- `check_infer` produces a type for every listed Exp form that
  currently falls through, matching what `check` accepts in checking
  mode.
- `subclass_of` inheritance works uniformly: a property declared on a
  parent class is accessible on instances of a child class.
- Type resolution is reasonably fast — memoize per `(class IRI, layer
  head)` so repeated lookups during a single check don't re-walk the
  chain.

**Non-goals:**

- Changing the ontology format or the `Class` / `Property` schema.
- EigenTT term-form changes. The work is in the checker; everything
  downstream accepts the better-typed output transparently.
- Universe stratification enforcement (that's Phase 10b — see
  life-science §16.2 and the Phase-10 plan).
- Full effect-system tracking. We only need Read-mode layer access at
  check time, not a richer capability system.
- Recovery from cycles in `subclass_of`. Existing validation rejects
  cyclic inheritance at ingestion; the checker can assume acyclic.

---

## 4. Layer access in the checker

### 4.1 The plumbing problem

`check_infer` / `check` / `check_type` take `(rho, gamma, exp)` — no
`Layer`. They're called from programs throughout the kernel:

- `program/expr.rs::parse_program` — has a `layer` handle.
- `program/eval_io.rs::execute_program_nbe_with_institutions` — has
  `layer`.
- `nbe/check::tests` — uses ad-hoc layers or `Val::One` placeholders.

The cleanest refactor is a small `CheckCtx` bundle threaded through
every checker call:

```rust
pub struct CheckCtx<'a> {
    pub rho: Rho,
    pub gamma: Gamma,
    /// Optional layer for ontology resolution. `None` is the
    /// "pure" case used by tests that don't touch EigonClass.
    pub layer: Option<Arc<Layer>>,
    /// Per-check memoization of resolved class types, keyed by
    /// `(class_iri, layer_head)`. Carried across the entire check so
    /// repeated property accesses on the same class hit the cache.
    pub type_cache: Arc<Mutex<BTreeMap<(String, LayerId), Val>>>,
}
```

Pros: one struct, explicit about what's needed, backwards-compatible
(`None` layer retains today's behavior for tests and pure checks).

Cons: invasive — every `check_infer` / `check` / `check_type` call
site has to adapt. Mitigated by adding an `Arc`-clone-friendly wrapper
and migrating call sites one at a time.

**Alternative (rejected):** plumb `Option<&Arc<Layer>>` as an extra
parameter to each function. More mechanical but leaves the cache
homeless and scales poorly as we add other per-check state
(stratification level tracking for 10b, inductive elaboration for 11).

### 4.2 Scoping

`CheckCtx` is constructed at the entry point (when the server parses a
program or validates a resource) and cloned cheaply into recursive
calls. The cache is intentionally per-check-entry: it covers a single
type-check pass, not a kernel-lifetime cache. A new program
type-check starts with a fresh cache; the per-call hit rate is high
enough within one pass because most real programs reference a small
set of classes repeatedly.

A kernel-wide cache is a future optimization — not needed for
correctness and introduces invalidation complexity on every layer
commit.

---

## 5. `find_sigma_field` with layer access

### 5.1 Algorithm

Given a type value `typ`, a field name, and a `CheckCtx`:

```
find_sigma_field(typ, field_name, ctx):
  case typ of
    Sig(t, g):
      if g.patt == Var(field_name): return Some(*t)
      else: continue walking the chain (unchanged)

    EigonClass(iri):
      let class_type = ctx.resolve_class_cached(iri)?
      find_sigma_field(&class_type, field_name, ctx)
      // Note: recursive call lets us chain through nested classes

    EigonPrimitive(_) | One | Set | Type(_):
      // Primitives don't have fields.
      None

    _: None
```

`resolve_class_cached` wraps `ground::resolve_class_type` with the
`CheckCtx.type_cache`:

```rust
impl CheckCtx {
    fn resolve_class_cached(&self, iri: &Iri) -> Result<Val, CheckError> {
        let layer = self.layer.as_ref()
            .ok_or(CheckError::NoLayerAccess(iri.clone()))?;
        let key = (iri.as_str().to_string(), layer.id().clone());
        if let Some(cached) = self.type_cache.lock().unwrap().get(&key) {
            return Ok(cached.clone());
        }
        let v = ground::resolve_class_type(iri, layer)
            .map_err(CheckError::ResolveClass)?;
        self.type_cache.lock().unwrap().insert(key, v.clone());
        Ok(v)
    }
}
```

### 5.2 Error messages

The current fall-through produces `"property X not found in type
{...}"` printing the Sigma dump. With the resolution landed, the
checker knows the class IRI and can produce a helpful error:

> property 'title' not found on class urn:eigenius:example:Patent
> (declared properties: abstract, filing_date, inventor, status)

This is the "errors that name the class and field" goal from §3.

### 5.3 Handling `subclass_of`

`ground::resolve_class_type` already handles inheritance via
`collect_properties_inner` — it walks `subclass_of` recursively and
unions the required + recommended sets. No extra work in
`find_sigma_field`; the Sigma chain it sees already includes inherited
fields.

### 5.4 Missing classes

If `resolve_class_cached` fails because the class isn't in the layer
chain (e.g., the user typed a property-access on a typo'd IRI), the
error bubbles up as:

> cannot resolve class 'urn:eigenius:example:Pattent' — not found in
> current layer (check spelling? class is declared before use?)

Deterministic failure, not a silent type promotion to `Set`.

---

## 6. `check_infer` extensions

Each missing form gets a rule. All straightforward; the work is
completing the match, not inventing new semantics.

### 6.1 `Construct(class_iri, fields)`

Type is the class itself:

```
check_infer(Construct(iri, fields), ctx) =
  let class_type = ctx.resolve_class_cached(iri)?
  // Verify each field against the class's Sigma chain — same logic
  // as check(Exp::Construct, Val::EigonClass(iri)) uses today.
  for (prop, e) in fields:
      let prop_typ = find_sigma_field(&class_type, prop.local_name(), ctx)
        .ok_or(...)?
      check(e, &prop_typ, ctx)?
  EigonClass(iri.clone())
```

Returns `EigonClass(iri)` so callers get the class type they'd expect
from a typed constructor.

### 6.2 `EigonResource(r)`

If the resource has an `is_a`, return that class:

```
check_infer(EigonResource(r), _) =
  match r.is_a().first():
    Some(class_iri) => EigonClass(class_iri.clone())
    None            => error "resource has no is_a class"
```

### 6.3 `Template(lit, refs)`

Template literals produce `core:string`:

```
check_infer(Template(_, refs), ctx) =
  // Each ref's type must be compatible with its declared interpolation type.
  for (iri, typ_exp) in refs:
      check_type(typ_exp, ctx)?
  EigonPrimitive(String)
```

### 6.4 `IdJ(args)`

The J-eliminator's type is the motive `C` applied to the endpoints:

```
check_infer(IdJ([a, c, d, x, y, p]), ctx) =
  let a_val = eval(a, ctx.rho)
  check_type(a, ctx)?
  check(x, &a_val, ctx)?
  check(y, &a_val, ctx)?
  check(p, &Id(a_val, eval(x, ctx.rho), eval(y, ctx.rho)), ctx)?
  // Motive c : (x y : A) → Id(A, x, y) → Set
  // d : (a : A) → c(a, a, refl(a))
  // Result: c(x, y, p)
  check_c_and_d(c, d, &a_val, ctx)?
  apply_motive(c, x, y, p, ctx)
```

This is the messiest case; punting the detailed typing rule to
nanoda_lib's `IdJ` handling (see `d28-lean-4-as-institution.md`).

### 6.5 `Refl(a)`

```
check_infer(Refl(a), ctx) =
  let a_type = check_infer(a, ctx)?
  let a_val  = eval(a, ctx.rho)
  Id(a_type, a_val, a_val)
```

### 6.6 `NativeDecide(constraint, v)` and `DecEq(a, x, y)`

Both reduce to a `Refl` in their success case. Their type is the
corresponding `Id`:

```
check_infer(NativeDecide(_, v), ctx) =
  let v_type = check_infer(v, ctx)?
  let v_val  = eval(v, ctx.rho)
  Id(v_type, v_val, v_val)

check_infer(DecEq(a, x, y), ctx) =
  check_type(a, ctx)?
  let a_val = eval(a, ctx.rho)
  check(x, &a_val, ctx)?
  check(y, &a_val, ctx)?
  Id(a_val, eval(x, ctx.rho), eval(y, ctx.rho))
```

### 6.7 Rejection

If a form in `check_infer` can't produce a type (e.g. `Refl(a)` where
`a` itself can't be inferred), the error names the specific blocker
rather than a generic *"cannot infer."*

---

## 7. Integration points

### 7.1 Callers of the checker

`program/expr.rs::parse_program` already owns a `layer`. It
constructs a `CheckCtx` with `layer = Some(Arc::clone(layer))` and
passes it through. The existing program flow is:

```
parse_program(prog_resource, layer) →
  body_exp = parse_expression(embedded_body, layer)
  ctx = CheckCtx::new_with_layer(Arc::clone(layer))
  check(&ctx, &body_exp, &typ_val)
```

### 7.2 Pure / ad-hoc tests

Tests that don't care about ontology resolution pass `layer = None`.
The new `find_sigma_field` falls back to today's behavior in the
`EigonClass` arm: emit a diagnostic or return `None` (since the
fallback `Val::Set` is what we're explicitly removing). Specifically:

```
EigonClass(_) if ctx.layer.is_none():
    Err(CheckError::NoLayerAccess(iri))
```

This surfaces the missing layer at every test that tried to rely on
the old silent fallback — a feature, not a regression. Each such test
gets either a real layer wired in or a test-local helper that stubs
one.

### 7.3 The `resolve_class_type` signature

`ground::resolve_class_type(iri, layer)` stays as is. `CheckCtx`
wraps it with caching; other callers (not the checker) can keep using
the uncached version. No changes to `ground.rs` required for §5.

---

## 8. Open questions

- **Non-ontology-class inference.** If a user writes
  `let x = Construct(SomeClass { ... })` where `SomeClass` isn't in the
  ontology, `Construct`'s inference rule errors out. Is that the right
  failure mode, or should we allow constructing "unvalidated"
  resources? *Proposal: reject. Construct is an ontology-typed form;
  unvalidated resources go through `EigonResource(literal)` instead.*

- **Property naming conflicts across inheritance.** If class `B`
  declares a `status` of type `String` and extends class `A` which
  declared `status` of type `Integer`, which one does property access
  resolve? `ground::resolve_class_type`'s current behavior is that
  derived classes' properties override parents. We should either
  confirm that or surface an ingestion-time error if a subclass
  shadows a parent property with a different type.
  *Proposal: reject at ingestion (subclass property must refine, not
  shadow-with-different-type). File a follow-up issue.*

- **Caching invalidation across layer commits.** The per-check cache
  scopes to a single type-check pass, so no invalidation needed. A
  cross-check cache would need invalidation on every `Load`-commit.
  *Proposal: skip the cross-check cache for v1. Revisit if profiling
  says the per-check cost dominates.*

- **Array / list typing.** A property declared `data_type:
  resource_array` with `class_types: [Patent]` should resolve to
  `List(EigonClass(Patent))` — but EigenTT doesn't have a native list
  type yet. `ground::resolve_array_element_type` returns the element
  type directly, which is lossy. *Proposal: Phase 11 tracks the list
  type; for 10a we match the existing behavior (return element type,
  accept the under-typing).*

- **Inference of `IdJ`'s motive.** The motive argument `C` in
  `J(A, C, d, x, y, p)` is hard to infer without bidirectional
  propagation. *Proposal: require an explicit motive in inference
  position. Inference mode rejects `IdJ` whose motive isn't a concrete
  `Lam` expression.*

---

## 9. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Layer access in checker | Bundle into a `CheckCtx` struct threaded through all checker calls | Extensible (stratification, inductive elaboration benefit later), explicit, backwards-compatible |
| Cache scope | Per-check-pass, keyed by `(class IRI, layer head)` | Correctness is automatic (fresh cache per pass); avoids cross-check invalidation |
| No-layer fallback | Error, not `Val::Set` | Silent type weakening is the bug we're fixing; explicit failure surfaces it in tests |
| `subclass_of` handling | Rely on existing `resolve_class_type` / `collect_properties_inner` | Already implemented; no redesign |
| Missing `check_infer` cases | All seven forms get rules matching their checking-mode behaviour | Completes the checker's inference surface |
| Construct without declared class | Reject | Keeps "typed construction" meaningful |
| Subclass property-type shadowing | Reject at ingestion (follow-up issue) | Type-safety — refinement only |
| List / array typing | Keep existing under-typing in 10a; full fix in Phase 11 | Avoids scope creep |
| `IdJ` motive inference | Require explicit motive in inference position | Full bidirectional inference is out of scope |

---

## 9a. Prior art

The "ontology-as-types" stance — a class *is* a dependent record type, an
instance *is* a record/witness of it — has direct external precedent in
type-theoretic semantics. Luo's common-nouns-as-types with coercive subtyping
and Chatzikyriakidis & Luo's MTT-semantics ([`chatzikyriakidis-luo-2020`])
give natural-language meaning over dependent record types and an impredicative
`Prop`; Barlatier & Dapoigny build ontologies directly on dependent record
types; and Cooper's TTR ([`cooper2023perception`]) is records-first, with a
record type's labelled fields-of-types matching an Eigenius class's
required/recommended properties exactly. These anchors are developed in detail
in D61 §10 and D62 §3, which this resolution's field-resolution machinery
shares a substrate with.

---

## 10. Implementation plan (for Phase 10a)

1. Introduce `CheckCtx` in `kernel/src/nbe/check.rs`, carrying `rho`,
   `gamma`, `Option<Arc<Layer>>`, and the per-check type cache.
   `CheckError` enum with variants `NoLayerAccess`, `ResolveClass`,
   `TypeMismatch`, `UnknownField`, etc.
2. Migrate `check`, `check_type`, `check_infer`, `check_decl` to take
   `&CheckCtx` instead of `(&Rho, &Gamma)`. This is the mechanical
   bulk — touch every call site.
3. Add `find_sigma_field` EigonClass arm: call
   `ctx.resolve_class_cached(iri)`, recurse into the returned Sigma.
   Remove the `Some(Val::Set)` fallback.
4. Add the seven missing `check_infer` cases (§6).
5. Update all callers of the checker (`program/expr.rs`,
   `program/eval_io.rs`, server handlers, unit tests) to construct
   `CheckCtx` with the appropriate layer.
6. Wire the ingestion-time "subclass can't shadow with different type"
   check (open question §8, follow-up issue once the issues list is
   filed).
7. **Tests:**
   - Property access on a resource typed by a Class with
     inherited properties resolves correctly and finds the parent's
     field (not just the child's).
   - Property access on an unknown class produces
     `CheckError::NoLayerAccess` with a useful message.
   - `let x = Construct(Patent { title = "…", … })` type-checks
     without annotation.
   - `let r = refl(42)` infers `Id(Integer, 42, 42)` without
     annotation.
   - Regression: every existing test that used the old `Val::Set`
     fallback either now threads a real layer through or gets a
     stubbed one, and still passes.
8. Performance sanity: profile a single `check` pass on the patent
   demo program, verify the cache is hit (most classes resolved
   exactly once per pass).

---

## 11. References

- `docs/design/d9-nbe-unification-and-type-extensions.md` §5.10 —
  EigenTT ground-type support; this is the "ontology-as-types" stub
  this document completes.
- `docs/design/life-science-requirements.md` §19 — sequencing that
  names this as Phase 10 step 1.
- `docs/design/life-science-requirements.md` §16.2 — universe
  stratification (Phase 10b, separate document forthcoming).
- `docs/design/d28-lean-4-as-institution.md` — nanoda_lib's IdJ and
  type-inference approach, used as a reference for §6.4.
- `kernel/src/nbe/check.rs` — `find_sigma_field`, `check_infer`,
  `check`, `check_type`, `check_decl`.
- `kernel/src/program/ground.rs` — `resolve_class_type`,
  `collect_properties_inner`, `build_sigma_chain`.
- eigenius/eigenius#12 — umbrella issue: High-priority correctness
  hazards (sigma-field resolution, check_infer completeness).
