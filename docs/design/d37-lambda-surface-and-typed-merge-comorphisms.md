# D37: Lambda surface and typed merge comorphisms

*Design document for the Eigenius project — May 2026*

**Status:** Implemented (ESL `merge_comorphism` / `lambda` / `pi` surface live; validator enforces EigenTT well-typedness)
**Builds on:** [D20 — Layer Reconciliation](d20-layer-reconciliation.md), [D36 — Merge Resolution UX](d36-merge-resolution-ux.md), [D7 — ESL Surface Syntax](d7-esl-surface-syntax.md), [D19 — Inductive Types](d19-inductive-types.md).
**Closes:** D36 §15's note that the witness happy path is gated on an authoring surface for EigenTT lambdas.

---

## 1. Overview

D20 / D36 give the user a typed witness surface for merge resolution: a `MergeComorphism` resource pins a transformation term of shape `(A, A, Option<A>) -> A`, and the kernel applies it at commit time to produce the merged body. The kernel-side apply path is wired end-to-end and unit-tested. What's missing is the **authoring surface** — there is no ergonomic way to write a witness transformation. The kernel's `MergeComorphism` resource and its embedded EigenTT lambda term are reachable today only by hand-rolling Eigon-JSON, which is fragile and unverifiable until merge time.

D37 closes that gap by adding three ESL surface forms:

1. **`lambda` expression form** — a EigenTT lambda literal usable inline in any expression position. Carries its parameters' types so the body is type-checkable from the literal alone.
2. **Standalone lambda resource declaration** — a top-level `resource ... : urn:eigenius:program:Lambda { ... }` form bound to a stable IRI, so a witness term can be authored, named, queried, and referenced by IRI from multiple sites.
3. **`merge_comorphism` declaration** — a first-class top-level form analogous to `program`, declaring the comorphism's domain class (`for A`) and either an inline lambda body or a reference to a separately-declared lambda resource. Compiles to the existing `urn:eigenius:core:MergeComorphism` resource shape plus the linked transformation.

The three forms compose: `merge_comorphism`'s inline body is sugar over "declare the lambda at a derived IRI and reference it" — same compile target either way. The factoring gives reuse across classes, addressability for tooling, and stable IRIs for versioning, without coupling the wrapper to the term.

### 1.1 Why now

D36 PR 2 shipped `WitnessEditor` against a kernel surface that fully supports witnesses but lacks authoring ergonomics. The result is that every witness-strategy resolution either hand-rolls Eigon-JSON or hits `MergeComorphismNotFound`. Each downstream institution that needs a typed function on the chain (cross-fibre `Comorphism`, future codata-stream transformers, decision-procedure handlers) will hit the same authoring wall and end up shipping its own ad-hoc surface. Lambdas are the chain's general typed-function representation; surfacing them once unblocks everything that builds on them.

### 1.2 Scope

D37 covers:
- ESL syntax for lambda literals, standalone lambda resources, and `merge_comorphism` declarations.
- The compiler's lowering to existing program-AST resource shapes — no new resource classes are introduced (the `Lambda` and `MergeComorphism` classes are already in the core ontology).
- Commit-time validation of standalone lambdas against their declared types.
- Commit-time validation of `merge_comorphism` declarations: the referenced (or inline) lambda's type must match `(A, A, Option<A>) -> A` where `A` is the `for` clause's class.
- A new `urn:eigenius:core:merge_target_class` well-known IRI on `MergeComorphism` resources, populated by the compiler from the `for` clause.
- Notebook UX changes that take advantage of `merge_target_class`: the `WitnessEditor` becomes a dropdown of applicable comorphisms rather than a free-form IRI input.

**Out of scope.**
- Polymorphic lambdas (type-level parameters). v1 of D37 ships monomorphic lambdas — `take_b` for `Patient` and `take_b` for `Visit` are two declarations. The polymorphism extension is sketched in §10.3 with its compile-side and validator-side gaps; the kernel's NbE machinery is already there (universes + Pi-binders over `Set`). The v1 → v2 boundary is strictly additive — every monomorphic witness committed in v1 remains valid in v2.
- Bounded quantification (`lambda<A extends ClassWithWeight>`). A consequence of unbounded polymorphism's shape-preserving restriction (see §10.3.4); requires its own design effort around subtyping or row-types.
- Pattern-matching ergonomic surface beyond what `program` already supports. `Match` over `Option` is needed for the ancestor argument and is already in the program AST; D37 reuses it without redesign.
- Visual witness-authoring UI in the notebook. The picker (Combobox of applicable comorphisms) is in scope; the builder is deferred to a separate design that revisits the broader resolution-strategy UX (see §10.6). That UX work is now scoped as **D39** — the D38 slot was reallocated to merge provenance + witness discovery scope (see [D38](d38-merge-provenance-and-witness-discovery.md)).
- Anything specific to the institution-layer `Comorphism` (the cross-fibre translation witness). That shares the lambda foundation but lives in its own surface design.

### 1.3 Expressiveness of witness bodies

The kernel's program AST already provides every term a witness body needs to construct arbitrary merged resources from `a`, `b`, and `opt`:

| Need | Existing AST node |
| --- | --- |
| Return one of the inputs unchanged | `Var` |
| Pull a field out of `a` or `b` (Σ-elimination) | `Project` |
| Build a new Σ-typed record by assigning fields | `Construct` |
| Branch on whether `opt` is `Some` or `None` | `Match` over `Option` |
| Numeric / string / boolean operations | `Apply` over chain-committed `Component` operators |
| Apply a constructor of an inductive type | `CtorApply` |
| Bind intermediate values | `Let` |
| Literal numbers / strings / booleans | `Literal` |

No new term shapes are required. The witness body is *the same* expression vocabulary `program` declarations already use; D37 is purely an authoring-surface change that emits the existing resource shapes. See §9 for worked examples covering each pattern.

---

## 2. Status today

The kernel side of the witness story is fully wired.

**Resource shapes (core ontology).** `urn:eigenius:core:MergeComorphism` (with required `merge_transformation` property) and `urn:eigenius:program:Lambda` (with `parameter` + `body` properties) are declared. The `parameter` is a string; the `body` is an embedded expression resource. Lambda's optional `type` slot is not yet declared — D37 adds it.

**Resolver.** `resolve_merge_comorphism` in [kernel/src/layer/merge.rs](../../kernel/src/layer/merge.rs) walks the merge span looking for the comorphism IRI, validates `is_a` includes `MergeComorphism`, extracts `merge_transformation`, returns a typed `MergeComorphismHandle`.

**Apply path.** `apply_witness_resolution` (same file) evaluates the transformation term against `(body_a, body_b, ancestor_body)` using the existing program evaluator, round-trips the resulting `ResourceVal` back into a `Resource`, and returns it for merge-layer construction. Currently the type-check is lazy (against the conflict class A inferred from the span); D37 makes it possible to validate at commit time instead.

**Server.** `submit_resolution`'s error mapping in [kernel/src/server/mod.rs](../../kernel/src/server/mod.rs) routes every kernel-side failure to the appropriate `SubmitResolutionErrorKind`. `APPLICATION_PENDING` is now a reserved wire value the kernel no longer constructs (the comment is explicit).

**Notebook side.** `WitnessEditor` (in [notebooks/src/components/merge/WitnessEditor.tsx](../../notebooks/src/components/merge/WitnessEditor.tsx)) renders a free-form comorphism-IRI input. There is no notebook-side affordance for **discovering** which comorphisms exist or **authoring** new ones — both are blocked on D37.

**What's missing.**
- ESL has no syntax for free-standing lambda terms.
- ESL has no syntax for `MergeComorphism` declarations.
- The validator has no commit-time type-check for standalone lambdas.
- `MergeComorphism` resources don't declare their target class — the kernel infers it at apply time.

---

## 3. ESL surface

Three new forms. The grammar additions in §3.4 collect them.

### 3.1 `lambda` expression literal

Available wherever an expression can appear (inside `resource` property assignments, on the right-hand side of `let`, as an argument to `Apply`, etc.):

```esl
// Inferred return type — the literal form most call sites use.
lambda a : project:Patient,
       b : project:Patient,
       opt : Option<project:Patient>
    => b

// Fully-annotated form — same shape with an explicit return-type
// suffix. Useful for forward-declared call sites and for cases where
// the body's inferred type isn't obvious from a glance.
lambda a : project:Patient,
       b : project:Patient,
       opt : Option<project:Patient>
    => b : project:Patient
```

The literal is a typed EigenTT lambda. The parameter list gives each binder its type; the body is an expression in the surrounding `lambda` scope. The optional `: <type>` suffix annotates the return type; when omitted, the validator infers it from the body. Allowed in **any expression position** — inside `resource` property assignments, on the right-hand side of `let`, as an argument to `Apply`, anywhere `program` declarations already accept embedded expressions.

Nesting is left-implicit: a single `lambda` with N parameters compiles to N nested single-parameter `Lambda` resource nodes (matching how the kernel's evaluator unpacks application chains).

### 3.2 Standalone lambda resource declaration

A top-level declaration that commits a lambda at a named IRI:

```esl
resource project:take_b_term : urn:eigenius:program:Lambda {
    lambda a : project:Patient,
           b : project:Patient,
           opt : Option<project:Patient>
        => b
}
```

Two structural notes:

1. The `: urn:eigenius:program:Lambda` class annotation makes the intent explicit and lets the validator reject mis-classed lambdas at commit time. The body must be a `lambda` literal.
2. The lambda's declared Pi-type is stored on the resource via a new `urn:eigenius:program:type` property carrying a serialised Pi-term. The validator type-checks the body against this Pi-term at commit time; the kernel's apply path can read it for early domain checks (§7.2).

### 3.3 `merge_comorphism` declaration

A first-class top-level declaration:

```esl
// Inline body — desugars to a synthesised standalone lambda at a
// derived IRI plus a MergeComorphism resource referencing it.
merge_comorphism project:patient_take_b for project:Patient {
    (a, b, opt) => b
}

// Reference form — references a previously-declared lambda. The
// referenced lambda's declared Pi-type must match
// `(Patient, Patient, Option<Patient>) -> Patient`.
merge_comorphism project:patient_take_b_ref for project:Patient {
    transformation = project:take_b_term
}
```

The `for A` clause is required. It carries two responsibilities:

1. **Domain identity.** Compiles to a `urn:eigenius:core:merge_target_class` property on the emitted `MergeComorphism` resource. The notebook surface uses this to populate the WitnessEditor's dropdown of applicable comorphisms; the kernel uses it to early-reject application to the wrong class.
2. **Type context for inline lambdas.** The inline body's parameter types are inferred from the `for` clause — the surrounding `for A` supplies `A` for the `a` and `b` parameters and `Option<A>` for `opt`. No annotations needed at the inline site; that's the ergonomic payoff.

### 3.4 Grammar additions

```ebnf
expression      ::= … existing forms … | lambda_literal
lambda_literal  ::= "lambda" lambda_params "=>" expression (":" type_expression)?
lambda_params   ::= typed_binder ("," typed_binder)*
typed_binder    ::= identifier ":" type_expression

declaration     ::= … existing forms … | merge_comorphism_decl
merge_comorphism_decl
                ::= "merge_comorphism" qname "for" qname "{"
                    merge_comorphism_body
                    "}"
merge_comorphism_body
                ::= inline_lambda_body | transformation_ref
inline_lambda_body
                ::= "(" untyped_params ")" "=>" expression
untyped_params  ::= identifier ("," identifier)*
transformation_ref
                ::= "transformation" "=" qname ";"?
```

`type_expression` covers existing ESL type positions (a class IRI, a primitive `core:*`, a parameterised inductive `Option<A>` / `List<A>`, or a `pi`-typed function). The `pi` form is also new but small — see §3.5.

### 3.5 `pi` type expression

Function types appear in `lambda` parameter annotations and on the `type` property of standalone lambda resources. The surface:

```esl
pi a : project:Patient,
   b : project:Patient,
   opt : Option<project:Patient>
  => project:Patient
```

Compiles to N nested single-parameter `Pi` AST nodes the same way `lambda` compiles to nested `Lambda` nodes. Most authors won't write `pi` by hand — it's emitted by the compiler from the `lambda` literal's parameter list. The standalone form is needed for forward-declared signatures and for the `type` property on standalone lambda resources.

---

## 4. Compiler lowering

### 4.1 `lambda` literal

ESL's existing `Expr::Lambda` AST node ([kernel/src/esl/ast.rs](../../kernel/src/esl/ast.rs)) — currently parsed from the untyped `\x -> e` surface — is **extended** rather than replaced: a new `param_type: Option<TypeExpr>` slot becomes `None` for untyped lambdas inside `program` bodies (where types are inferred from context) and `Some(T)` for the new typed surface (`lambda x : T => body`). The existing untyped form continues to work unchanged. A parallel `TypeExpr::Pi` variant is added for the `pi` type expression in §3.5 — `TypeExpr` previously had `Ref`, `Arrow`, and a size-binder-specific `BinderArrow`; the new `Pi` variant is the general value-typed binder. Both deltas are additive; every existing match-site picks up a wildcard arm.

A `lambda x_1 : T_1, …, x_N : T_N => body` lowers to a right-associated nested chain of `urn:eigenius:program:Lambda` resources:

```
Lambda {
  parameter = "x_1",
  type      = <T_1 as Pi-term>,        -- new in D37
  body      = Lambda {
    parameter = "x_2",
    type      = <T_2 as Pi-term>,
    body      = …
                Lambda {
                  parameter = "x_N",
                  type      = <T_N as Pi-term>,
                  body      = <compiled body>
                }
  }
}
```

The lambda's overall type is the Pi-term `pi x_1 : T_1, …, x_N : T_N => <return-type>`. For standalone lambda resources the Pi-term is stored alongside the lambda on `urn:eigenius:program:type`. For inline literals it's reconstructed at type-check time and not materialised.

### 4.2 Standalone lambda declaration

`resource <iri> : urn:eigenius:program:Lambda { <lambda-literal> }` lowers to:

- A top-level resource at `<iri>` with `is_a = [urn:eigenius:program:Lambda]`.
- The body lambda's properties (`parameter`, `type`, `body`) copied onto the top-level resource directly (matching the kernel test's `make_lambda_resource` pattern — the outermost Lambda is the resource itself, not an embedded child).
- A `urn:eigenius:program:type` property carrying the Pi-term reconstructed from the literal's parameter annotations.

### 4.3 `merge_comorphism` declaration

`merge_comorphism <iri> for <class> { <body> }` lowers to either one or two resources:

**Reference form.** One resource at `<iri>`:
```
{
  "@id": "<iri>",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:MergeComorphism"],
  "urn:eigenius:core:merge_transformation": "<referenced-lambda-iri>",
  "urn:eigenius:core:merge_target_class": "<class>"
}
```

**Inline form.** Two resources:
1. A synthesised standalone lambda at `urn:eigenius:auto:lambda:<sha256>`, where `<sha256>` is the content-hash of the lambda's canonical Eigon-CBOR shape (matching the existing anchored-commit dedup convention). The synthesised lambda's Pi-type is constructed from the surrounding `for` clause: `(class, class, Option<class>) -> class` (where the return type is the inline body's inferred type — typically `class`).
2. A `MergeComorphism` resource at `<iri>` as in the reference form, pointing at the synthesised lambda.

The content-hash IRI gives free deduplication: re-declaring the same inline body produces the same hash, so the anchored-commit cache short-circuits and no duplicate resource is committed. The `urn:eigenius:auto:` namespace is reserved for compiler-synthesised IRIs that the user is **not** expected to read or reference by hand — they surface only in audit views like the Layer Inspector.

---

## 5. Validator changes

D37 adds two commit-time validations.

### 5.1 Lambda well-typedness

When a resource with `is_a ⊇ [urn:eigenius:program:Lambda]` is committed, the validator:

1. Reads `urn:eigenius:program:type` — required.
2. Walks the lambda's nested-Lambda chain alongside the Pi-term, unifying parameter binders.
3. Type-checks the innermost body against the Pi-term's return type using the existing NbE infrastructure.

Standalone lambdas with no `type` property are rejected with `MissingLambdaType` — embedded lambdas inside `program` bodies (where types are inferred from surrounding context) are exempt because they're not top-level resources and don't enter this validation path.

### 5.2 MergeComorphism shape

When a resource with `is_a ⊇ [urn:eigenius:core:MergeComorphism]` is committed, the validator:

1. Reads `merge_target_class` — required (this is the new well-known property).
2. Reads `merge_transformation` — required, must be a `ResourceRef` (existing check).
3. Resolves the referenced lambda by IRI.
4. Reads the lambda's `type` Pi-term.
5. Verifies the Pi-term is exactly `(A, A, Option<A>) -> A` where `A = merge_target_class`.

Failures surface as new typed errors: `MergeComorphismMissingTargetClass`, `MergeComorphismTransformationTypeMismatch { expected, actual }`. Both map to `MalformedResolution` on the SubmitResolution wire (no new wire enum values needed).

### 5.3 No legacy compatibility

Eigenius hasn't shipped. Every `MergeComorphism` committed by the validator after D37 lands must carry `merge_target_class`; the validator rejects any resource missing it with `MergeComorphismMissingTargetClass`. There is no fall-back to the pre-D37 lazy-check at apply time. Any hand-rolled witnesses authored before D37 (via Eigon-JSON cells or the CLI) must be re-committed through the new ESL surface or amended to carry `merge_target_class`.

---

## 6. Kernel changes

### 6.1 New well-known IRI

```rust
// kernel/src/ontology/well_known.rs
pub const MERGE_TARGET_CLASS: &str = "urn:eigenius:core:merge_target_class";
pub const PROGRAM_TYPE: &str = "urn:eigenius:program:type";
```

`merge_target_class` joins `merge_transformation` on the `MergeComorphism` class. `program:type` is the Pi-term slot on `Lambda` resources.

### 6.2 Apply-time domain check (early-reject)

`resolve_merge_comorphism` gains a class-equality check after locating the comorphism: if the comorphism's `merge_target_class` doesn't match the conflict's class `A`, return `MergeComorphismWrongClass { expected: A, actual: <target_class> }`. Currently the type mismatch surfaces deep inside `apply_witness_resolution`'s evaluator and the error message is opaque; the new early check produces a clean, actionable error.

### 6.3 Core-ontology updates

The core ontology JSON ([ontologies/core/core-ontology.json](../../ontologies/core/core-ontology.json)) gains:

- A `Property` resource for `urn:eigenius:core:merge_target_class` (data_type `urn:eigenius:core:resource`, class_types `[urn:eigenius:core:Class]`).
- The `MergeComorphism` class adds `merge_target_class` to its `requires` list.
- A `Property` resource for `urn:eigenius:program:type` (data_type `urn:eigenius:core:resource`).
- The `Lambda` class adds `type` to its `recommends` list (embedded lambdas don't need it; standalone lambdas do — the validator enforces presence for the standalone case).

---

## 7. Notebook UX downstream

Two improvements light up once `merge_target_class` is on the chain.

### 7.1 WitnessEditor — comorphism picker

Today's free-form IRI input becomes a Combobox of applicable comorphisms. On mount, the editor extracts the conflict's class and fires an EigenQL query against the chain for matching comorphisms:

```eigenql
USING "urn:eigenius:core:MergeComorphism"

MATCH MergeComorphism(?c) {
    "urn:eigenius:core:merge_target_class": ?cls
}
WHERE ?cls = "<conflict-class-iri>"
RETURN [] {
    iri: ?c
}
```

**Extracting the conflict's class.** `TypedConflictWire`'s variants don't carry the class IRI directly (`IriCollisionConflict` has `iri` + the two `branch_*_body_json` payloads but no `class` field), so the editor pulls it from `branchABodyJson` — parse the JSON, read `urn:eigenius:core:is_a[0]`, that's the class IRI to filter on. No wire-format change required; the body is already present on the wire for diff rendering. For `PropertyDataTypeConflict` and `KindMismatchConflict` the same body-extraction approach works.

The Combobox renders each result with its short name + IRI tooltip. An empty result list keeps the free-form input as a fall-through ("no comorphisms exist for this class — paste an IRI to bind one ad-hoc").

### 7.2 Discoverability surfaces

The same query underlies a future **Comorphisms** rail destination — list all witnesses on the chain, grouped by target class, with their lambda bodies' types visible. Out of scope for D37's first cut but the query shape is the same.

---

## 8. Phasing / rollout

Four PRs. Each individually shippable; the kernel changes (§5–6) land first so subsequent PRs can rely on the new shapes.

### PR 1: Kernel foundation + apply-time enforcement — **SHIPPED**

What landed:

- New well-known IRIs (`MERGE_TARGET_CLASS`, `PROGRAM_TYPE`).
- Core ontology JSON updated: `merge_target_class` Property declared and added to `MergeComorphism.requires`; `program:type`'s domain extended to include `Lambda`; `Lambda.recommends` gains `program:type`.
- `resolve_merge_comorphism` gains an `expected_class` parameter and an early-reject path returning `MergeError::MergeComorphismWrongClass` on a class mismatch (plus `MalformedMergeComorphism` for the missing-property case).
- Existing kernel `merge.rs` fixtures migrated to include `merge_target_class` (the §10.2 strict-mode side effect; ~5 call sites). This was originally scoped to PR 4 but had to land alongside the resolver change to keep tests green.
- Two new unit tests pinning the early-reject flow.

Realised effort: ~2 days. Slightly under the 3–4 day estimate because two pieces moved out (see below).

**What moved to PR 2 (commit-time validators).** Lambda well-typedness and MergeComorphism shape are gated on PR 2's ESL surface — every Lambda in the chain today is type-free (no `program:type` populated), so commit-time validation would either reject every existing Lambda or have to skip them. The validators land alongside the ESL surface that actually produces typed lambdas. The wire surface and apply-time enforcement are fully in place now; commit-time pinning waits until there's something to pin.

### PR 2: ESL syntax + commit-time validators

- AST nodes: extend `Expr::Lambda` with an optional `param_type`; add `TypeExpr::Pi` variant; new `MergeComorphismDecl` declaration node.
- Parser rules per §3.4.
- Compiler lowering per §4 (including the inline `merge_comorphism` body sugar that emits the synthesised standalone lambda at a content-hash IRI).
- **Commit-time validators (folded in from PR 1):**
  - Lambda well-typedness — when a standalone Lambda carries `program:type`, the validator type-checks the body against the declared Pi-term.
  - MergeComorphism shape — `merge_target_class` required; the referenced (or synthesised) transformation's Pi-type must match `(A, A, Option<A>) -> A` where A is the target class.
- ESL tests pinning the resource bytes emitted by each form (round-trip via `compile + load + inspect`).
- Validator unit tests for each commit-time rejection path.

Estimated effort: ~9–11 days (up from 7–9 because the commit-time validators moved in from PR 1). Still the largest piece. The parser delta is small; lowering, type-elaboration, and the new validators are where most of the work goes.

### PR 3: Notebook WitnessEditor — comorphism picker

- Replace WitnessEditor's free-form input with the EigenQL-driven Combobox.
- Add a body-extraction helper that pulls the class IRI out of `branchABodyJson` (§7.1).
- Empty-list fallback to the free-form input.
- Live updates when the chain advances (re-fire the query on tip change).

Estimated effort: ~2.5 days. Mostly UI; the EigenQL surface is already in place.

### PR 4: Test scenario conversion

- Update D36 manual test Scenario 6 to exercise the happy path (commit a witness via the new ESL surface, drive a successful Witness resolution).
- Add ESL-side unit tests for worked examples §9.1–§9.4.

Estimated effort: ~1 day (down from 1.5 — the fixture migration moved into PR 1's actual landing).

### Total

Roughly **14–17 days** of focused work, ~3 weeks calendar. PR 1 is in; PR 2 is the long pole. PRs 3 and 4 are independent of one another and can land in either order once PR 2 ships.

---

## 9. Worked examples

Each example is a concrete witness body that the surface can author. The first three are monomorphic on `project:Patient` (a Σ-typed class with `description : core:string` and `weight : core:float`); the fourth is a sketch of an inductive-aware pattern.

### 9.1 Take side B

```esl
merge_comorphism project:patient_take_b for project:Patient {
    (a, b, opt) => b
}
```

Body: `Var "b"`. Lowers to a single-Var lambda body. Simplest possible witness — useful as a primitive and as the test fixture for §11.

### 9.2 Field-merge: take A's description, B's weight

```esl
merge_comorphism project:patient_merge_fields for project:Patient {
    (a, b, opt) => Construct project:Patient {
        project:description = a.description,
        project:weight      = b.weight,
    }
}
```

Uses `Construct` (Σ-introduction) + `Project` (Σ-elimination via the `.field` notation, which compiles to a `Project` node). The kernel evaluator handles this trivially today.

### 9.3 Take the average weight (arithmetic)

```esl
merge_comorphism project:patient_avg_weight for project:Patient {
    (a, b, opt) => Construct project:Patient {
        project:description = a.description,
        project:weight      = core:divide(core:add(a.weight, b.weight), 2.0),
    }
}
```

`core:add` and `core:divide` are chain-committed `Component` operators. `Apply` nodes invoke them; no new term shape needed.

### 9.4 Ancestor-aware: only diverge when ancestor matches A

```esl
merge_comorphism project:patient_ancestor_aware for project:Patient {
    (a, b, opt) => match opt {
        Some(ancestor) =>
            if a.weight == ancestor.weight
                then b   -- A didn't change weight; safe to take B
                else a,  -- A changed; prefer A's weight
        None =>
            a            -- no ancestor; default to A
    }
}
```

Uses `Match` over `Option<Patient>` and a (chain-committed) `core:equals` `Component` via `Apply`. The `if-then-else` shape compiles to a `Match` on `core:Bool`'s `True`/`False` constructors. All terms exist today.

A full type-checker walkthrough for §9.4 is in [appendix A] (TBD).

---

## 10. Resolved decisions and forward references

Decisions resolved during the design review. Each is folded into the relevant body section above; this log records the rationale.

### 10.1 Synthesised-lambda IRI derivation — content hash

**Decided:** `urn:eigenius:auto:lambda:<sha256>` over the lambda's canonical Eigon-CBOR shape. Folded into §4.3.

The content-hash form gives free deduplication and avoids collisions with user-authored names. These IRIs are not expected to be referenced by hand — they surface only in auditing views (Layer Inspector, History panel) where the namespace prefix `urn:eigenius:auto:` is the cue that the resource was synthesised by the compiler.

### 10.2 Legacy MergeComorphism compatibility — no compat

**Decided:** strict — every `MergeComorphism` must carry `merge_target_class`. Folded into §5.3.

Eigenius hasn't shipped. There are no chains to be backward-compatible with; adding a permissive fall-back path just maintains two type-check codepaths in perpetuity for no benefit.

### 10.3 Polymorphism — sketch and gaps

**Decided:** v1 of D37 ships monomorphic lambdas. Polymorphism is a separate increment that extends — does not restructure — the surface and validator. The kernel-side machinery is already in place; the gap is at the surface and elaboration layers.

#### 10.3.1 Surface extension

The polymorphic form adds a type-parameter list before the value-parameter list:

```esl
// Standalone polymorphic lambda — one identity-style witness reusable
// across every class.
resource project:take_b_term : urn:eigenius:program:Lambda {
    lambda<A> a : A, b : A, opt : Option<A> => b
}

// Polymorphic Pi-type for the same lambda's `type` property:
pi<A : Set>. pi a : A, b : A, opt : Option<A> => A
```

`A` is bound at universe `Set` (the type of types), which the kernel's NbE already represents as `Val::Set` ([kernel/src/nbe/val.rs:39](../../kernel/src/nbe/val.rs#L39)). Grammar delta is small: an optional `<…>` after the `lambda` / `pi` keyword introducing type-parameter binders. The body's value parameters can now reference those binders in their type slots.

At a use site, the type parameter is **inferred from the `for` clause**:

```esl
// `for project:Patient` unifies A := project:Patient when resolving
// the transformation's polymorphic Pi-type against the expected
// `(Patient, Patient, Option<Patient>) -> Patient`.
merge_comorphism project:patient_take_b for project:Patient {
    transformation = project:take_b_term
}
```

Explicit instantiation (`transformation = project:take_b_term<project:Patient>`) is also accepted but optional — the inference is unambiguous given the `for` clause.

#### 10.3.2 Compile-side gaps (additive)

- ESL parser accepts the `<…>` binder list after `lambda` / `pi`. Small grammar delta.
- AST nodes carry an optional `type_params` vector alongside the existing value-parameter list.
- Compiler emits an outer chain of Pi-over-`Set` binders before the value binders, and an outer chain of Lambda-over-`Set` binders on the lambda value. The kernel evaluator already handles these — they're just Pi/Lambda nodes whose parameter type is `Set` instead of a class.

#### 10.3.3 Validator-side gaps (additive)

- The commit-time type-check elaborates the polymorphic Pi-term as it walks the lambda's nested chain, threading the binder context through. This is mechanical — the existing NbE elaborator in `kernel/src/nbe/check.rs` already supports universe binders for the `program` declaration form (Phase 11d's institution Comorphisms exercise this path).
- The MergeComorphism shape check (§5.2) needs one extension: when the comorphism's `transformation` references a polymorphic lambda, the validator unifies the lambda's outer type-parameter with the comorphism's `merge_target_class`. If unification succeeds, the remaining check (matching against `(A, A, Option<A>) -> A`) proceeds as in the monomorphic case.

#### 10.3.4 Structural limitation: shape-preserving bodies

Polymorphic witnesses **can't access the Σ-record structure of their parameters**. The body type-checks against an unconstrained `A`; the validator doesn't know `A` has a `weight` field, so expressions like `a.weight` or `Construct A { weight = … }` are rejected. Polymorphic lambdas are therefore limited to bodies that:

- Return a parameter as-a-whole (`Var "a"`, `Var "b"`).
- `Let`-bind locally.
- `Match` over the `Option<A>` parameter (whose constructor structure *is* known, since `Option` is concrete).
- Apply non-class-specific operators (e.g., a `core:assert_eq` that only requires `core:Eq`-class evidence).

Concretely: the worked examples in §9.1 (take-B) and §9.4 (ancestor-aware fall-through that doesn't peek into ancestor's fields) work polymorphically. §9.2 (take A's description + B's weight) and §9.3 (average the weights) don't — they need to know the fields exist, which means the lambda must be monomorphic.

The fix for the structural limitation is **bounded quantification** — `lambda<A extends ClassWithWeight>` — which is a non-trivial extension to the type theory (introduces subtyping or row-typed records). That's its own design effort. v2 of D37 ships unbounded polymorphism with the shape-preserving restriction; v3 (or a separate design entirely) tackles bounded quantification if real workloads need cross-class field-level merges.

#### 10.3.5 Why we know we won't paint ourselves into a corner

The monomorphic v1 surface is a strict subset of the polymorphic v2 surface — a monomorphic `lambda a : A, b : A, opt : Option<A> => b` is the polymorphic `lambda<A> a : A, b : A, opt : Option<A> => b` with `A` immediately specialised. Every monomorphic witness committed in v1 remains valid in v2, just with no further type-parameter inference happening. The kernel's NbE elaborator already discriminates universe-bound binders from value-bound binders cleanly; the ESL surface delta is purely additive (the `<…>` is optional). No restructure is required at the v1 → v2 boundary.

### 10.4 Return-type annotations on inline `lambda` — included in v1

**Decided:** the optional `=> body : <type>` suffix lands in v1. Folded into §3.1 and the grammar in §3.4.

The annotation is mechanically simple — the parser accepts a trailing `: <type_expression>` after the body, and the validator uses the annotation as the expected return type for body type-checking instead of inferring. Includes a useful diagnostic (`LambdaReturnTypeMismatch { declared, actual }`) when the user's annotation disagrees with the body.

### 10.5 Where to allow the `lambda` literal — permissive

**Decided:** any expression position. Folded into §3.1.

Every existing `program` declaration already supports embedded lambdas in its body. The literal form merely surfaces what the AST already accepts; restricting placement would invent a constraint that doesn't exist in the underlying term language.

### 10.6 Notebook authoring surface — ESL for v1; comprehensive UX deferred

**Decided:** D37 v1 ships only the **picker** UI for selecting already-committed witnesses (§7.1). Authoring goes through ESL cells. A richer visual builder is deferred and will land alongside a broader redesign of the resolution-strategy UX.

The current merge UI has six strategy radios with kind-specific editors and is already non-trivial to reason about as the conflict-kind taxonomy grows. Layering a witness-construction builder on top of the current shape would compound the complexity. A subsequent design (provisionally **D39 — Resolution strategy UX, second pass**; the D38 slot is taken by [D38 — Merge provenance and witness discovery](d38-merge-provenance-and-witness-discovery.md)) should consolidate:

- Strategy applicability surfacing (when each strategy makes sense given the conflict kind, beyond the current "greyed-out" hint).
- Per-strategy authoring affordances — including a visual witness builder that emits ESL into a sibling cell.
- Discovery surfaces for chain-committed witnesses, comorphisms, and rename targets that today require knowing IRIs by hand.

D37 stays narrow on the foundation (typed lambdas + typed merge comorphisms); the wider UX evolves separately on top of it.

---

## 11. Validation plan

Per PR.

**PR 1.** Kernel unit tests:
- Standalone lambda with correct `type` property — accepted.
- Standalone lambda with `type` body-mismatch — rejected with `LambdaBodyTypeMismatch`.
- Standalone lambda missing `type` — rejected with `MissingLambdaType`.
- `MergeComorphism` missing `merge_target_class` — rejected.
- `MergeComorphism` with transformation type not matching `(A, A, Option<A>) -> A` — rejected with `MergeComorphismTransformationTypeMismatch`.
- `resolve_merge_comorphism` returns `MergeComorphismWrongClass` on a class mismatch (independent of apply path).

**PR 2.** ESL tests:
- `lambda` literal round-trips through compile → commit → inspect.
- Standalone lambda declaration produces the right resource bytes.
- `merge_comorphism` inline form emits both resources at the expected IRIs.
- `merge_comorphism` reference form emits one resource pointing at an existing lambda.
- Each worked example (§9.1–§9.4) compiles cleanly and round-trips.

**PR 3.** Notebook tests:
- `WitnessEditor` shows a Combobox populated by the chain's `MergeComorphism` resources for the current conflict's class.
- Empty result falls back to the free-form input.
- Picking a comorphism produces the right `MergeResolutionWire` payload.

**PR 4.** End-to-end:
- D36 §test-scenario 6 (now happy-path Witness) drives a full resolution through the new ESL surface and the new editor. Asserts a successful merge layer.

---

## 12. References

- [D7 — ESL Surface Syntax](d7-esl-surface-syntax.md) — base grammar D37 extends.
- [D19 — Inductive Types](d19-inductive-types.md) — Option's chain shape, the parametric-inductive surface D37's type expressions hook into.
- [D20 — Layer Reconciliation](d20-layer-reconciliation.md) §6.1 — the witness contract D37 makes authorable.
- [D36 — Merge Resolution UX](d36-merge-resolution-ux.md) §15 — the test-scenario 6 deferment D37 closes.
- `kernel/src/program/expr.rs` — the program AST resource shapes D37's compiler emits.
- `kernel/src/layer/merge.rs` — `resolve_merge_comorphism` + `apply_witness_resolution` — the kernel paths D37 wires into.
