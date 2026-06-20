# D59 — EigenQL: derived-relation joins + array element patterns (plan)

*Status: **implemented** (Items 1–3) · June 2026 · unblocks D58 Reachable gate, then D57/D58 resume*

*Implementation summary: three defects fixed + array patterns added, all in
`kernel/src/query/`, verified by 7 new unit tests (1565 kernel tests green, clippy
clean) and a live kernel run of the Reachable query against the D58 objective graph
(`m_orphan` correctly flagged). A **third** latent defect surfaced during Item 3
validation — see §1b.*

*This work was surfaced by the D58 dogfood: building the objective-graph
**Reachable** well-posedness check (transitive closure over `objective:depends_on`)
hit three concrete EigenQL defects/gaps. Two are a single evaluator bug; one is a
missing pattern-language primitive. Both must land before Reachable is expressible,
so this is sequenced ahead of finishing D57/D58.*

---

## 0. Motivation & witnessed evidence

Reachable needs two things EigenQL currently can't do:
1. **Join a `DEFINE`-relation's bound variable back to a resource's properties**
   ("for each reached node `?m`, follow `?m`'s `depends_on`").
2. **Iterate the elements of an array-valued property** (`depends_on` is a
   `resource_array`; today a pattern binds the whole array, not its elements).

Witnessed on branch `obj-d57` (objective graph: thesis → m1..m4 → axioms):

| Query form | Expected | Got |
|---|---|---|
| `"Milestone"(?m) { depends_on: ?arr }` (concrete) | 5 rows | **5** ✓ |
| `R(?m) {}, ?m { depends_on: ?arr }` (derived + free-var brace) | 5 | **30** (6×5 cross-product) |
| `R(?m) {}, "Milestone"(?m) { depends_on: ?arr }` (derived + class re-match) | 5 | **30** |
| `R(?m) { depends_on: ?arr }` (the guide's `Ancestor(?y){…}` idiom) | 5 | **0** |
| `{ depends_on: ?t }` (array binding) | per-element | binds whole array |
| `MATCH R(?x) {} RETURN { x: ?x }` (project derived subject) | rows | "unbound variable ?x" |

## 1. Item 1 (prerequisite) — fix derived-relation binding & join

**Symptom:** a variable bound by a `DEFINE` relation does not carry into the rest
of the query — it cross-products with later patterns, is empty under single-pattern
brace refinement, and is unbound in `RETURN`.

**Root cause (found):** derived-relation rows are materialized as pseudo-resources
with `resource_iri = None` and their columns stored as `urn:derived:{col}`
properties ([`evaluate/pattern.rs::collect_candidates`](../../kernel/src/query/evaluate/pattern.rs) lines ~99–117). But
[`try_match_resource`](../../kernel/src/query/evaluate/pattern.rs) binds the
pattern's **subject only from `resource_iri`** — which is `None` for these — so the
relation's head variable is never bound (dropped), and the brace's property lookups
hit the pseudo-resource (`urn:derived:*`) instead of the real resource. One defect,
all three symptoms.

**Fix (approach):** when a derived-relation column value is a resource IRI, resolve
it to the **real resource** so the pattern matches uniformly with concrete
candidates:
- In `collect_candidates`' derived branch: for the pattern's subject column, set
  `resource_iri = Some(<the IRI value>)` and `props = layer.resolve(iri).properties()`
  (fall back to the pseudo-prop value when the column is a literal, not an IRI).
  Extract the IRI with **`Value::as_iri()`**, which accepts both `Value::String`
  (parse-time) and `Value::ResourceRef` (the chain-canonicalised shape). *(Found in
  D57 m2: a strict `Value::String` match silently dropped a derived subject read
  from a resource-valued property — e.g. `Reach(?t) FROM Objective { thesis: ?t }`
  where `thesis` canonicalises to `ResourceRef` — making the whole graph
  unreachable. Regression test: `derived_subject_from_resource_ref_property_resolves`.)*
- Ensure `try_match_resource` binds **every** head variable of the relation
  (not just the subject) from the derived row, with the existing `values_equal`
  consistency check so joins constrain rather than cross-product.
- Confirm `RETURN`/projection sees derived-bound variables
  ([`evaluate/return_shape.rs`](../../kernel/src/query/evaluate/return_shape.rs)).

**Scope decision:** the parser supports only single-subject relation patterns
(`R(?m)`), not multi-arg (`Edge(?m, ?n)` → parse error today). The single-subject +
brace-refinement model is sufficient for Reachable once this fix lands, so
**multi-arg relation patterns are out of scope** (note it; revisit only if a future
query needs a 2-column join no resource property can carry).

**Investigate / modify:**
- `kernel/src/query/evaluate/pattern.rs` — `collect_candidates` (derived branch),
  `try_match_resource` (subject + head-var binding). *(primary)*
- `kernel/src/query/evaluate/mod.rs` — DEFINE seminaive fixpoint (lines ~83–106):
  confirm derived bindings store IRIs in the form `values_equal` matches.
- `kernel/src/query/evaluate/return_shape.rs` — derived-var projection.
- `kernel/src/query/ast.rs` — `Pattern` shape (no change expected; confirm).

**Tests (new, `kernel/tests/` or `#[cfg(test)]` in `evaluate/pattern.rs`):**
- derived bind → property refinement returns the joined rows (not a cross-product);
- `DEFINE R … MATCH R(?x) {} RETURN {x}` projects;
- a recursive 1-arg relation over a real property converges (Ancestor-style);
- regression: the documented `Ancestor(?y) { reports_to: ?z }` idiom now works.

## 1b. Item 1b (discovered during Item 3) — stratified negation must evaluate in stratum order

**Symptom:** the Reachable query (`Unreachable = Node \ Reach`, a negation over a
recursive relation) reported one *extra* unreachable node — a node that **is**
reached was flagged.

**Root cause:** `evaluate/mod.rs` ran *all* `DEFINE` rules together in a single
monotonic add-only fixpoint. `Unreachable` negates `Reach`, so it was computed
against a **partial** `Reach` in early iterations and added stale rows (e.g. `ax`,
before the recursion reached it) that the add-only loop never retracts. The
stratifier (`query/stratify.rs`) already computes ordered strata and `query/mod`
calls it — but only to *validate*; the order was discarded and the documented
"run stratum 0 to fixpoint, then stratum 1" semantics (guide ch.10) were never
implemented.

**Fix:** `evaluate/mod.rs` now evaluates **stratum by stratum in order**, running a
seminaive fixpoint over each stratum's rules with all lower strata already fixed.
Negation in a higher stratum therefore sees a complete lower relation. (Within a
stratum only positive recursion is allowed — the stratifier guarantees it — so the
add-only fixpoint is sound there.)

**Modified:** `kernel/src/query/evaluate/mod.rs` (uses `stratify::stratify`'s
ordered output). Verified by `reachable_gate_well_posed_graph_has_no_unreachable`
(0) and `reachable_gate_flags_orphan` (1).

## 2. Item 2 — array element-iteration + cardinality patterns

**Gap:** `{ p: ?v }` on an array-typed property binds the whole array; there's no
way to bind/iterate elements. Add array patterns in the property-object position:

| Form | Meaning |
|---|---|
| `{ p: [] }` | `p` is the empty array (cardinality 0) |
| `{ p: [?a] }` | exactly one element, bound to `?a` |
| `{ p: [?a, ?b] }` | exactly two elements, positional bind |
| `{ p: [?a, ?b, ...] }` | at least two; `?a`,`?b` = first two, rest ignored |
| `{ p: [... ?e ...] }` | **iterate** — one binding per element (the unnest primitive) |

`[... ?e ...]` is the load-bearing one (turns an array property into a relation);
the cardinality forms are structural matches. Chain arrays are ordered `Vec`s, so
positional binding is well-defined; iteration is a binding-multiplying positive
join (stratification unaffected).

**Investigate / modify:**
- `kernel/src/query/lexer.rs` — `[`, `]`, and `...` (ellipsis) tokens (brackets
  likely already lexed for array literals; confirm `...`).
- `kernel/src/query/parser.rs` — parse an array pattern in `PropertyPattern.object`
  (the property-value position); distinguish from the existing array *literal* in
  expressions.
- `kernel/src/query/ast.rs` — extend `ValueOrVariable` (or add an `ArrayPattern`
  node) with the five forms (`Empty`, `Exact(vec)`, `AtLeast(vec)`, `Each(var)`).
- `kernel/src/query/evaluate/pattern.rs` — `try_match_resource`: when the object is
  an array pattern, match against the `Value::Array`: cardinality check + positional
  bind, or element iteration (emit one binding per element, joining on shared vars).
- `kernel/src/query/type_check.rs` — the property must be `resource_array` /
  `value_array`; element variables typed at `class_types` / `element_type`; reject
  array patterns on scalar properties.
- `kernel/src/query/stratify.rs` — confirm no change (iteration is positive).

**Tests:** each of the five forms (empty/exact-1/exact-2/at-least-2/each); type
errors on scalar properties; `[... ?e ...]` produces N bindings; composition with
recursion.

## 3. Item 3 — validate Reachable end-to-end, resume D58/D57

With Items 1+2, Reachable is clean and correct:
```eigenql
DEFINE Reach(?t) FROM MATCH "…:Objective"(?o) { thesis: ?t }
DEFINE Reach(?n) FROM MATCH Reach(?m) { "…:depends_on": [... ?n ...] }   -- joins (Item 1) + iterates (Item 2)
DEFINE Node(?x) FROM MATCH "…:Milestone"(?x) {}
DEFINE Node(?x) FROM MATCH "…:Axiom"(?x) {}
DEFINE Unreachable(?x) FROM MATCH Node(?x) {}, NOT Reach(?x) {}
MATCH Unreachable(?x) {} RETURN [] { x: ?x }
```

**Acceptance:**
- well-posed objective ⇒ `Unreachable` empty;
- the orphan-milestone negative ⇒ orphan flagged (the test that's currently red);
- dangling edge (a `depends_on` IRI with no resource) and cycle (`Reach(?x)`
  reaching itself) variants behave;
- then D58 §5.4: register Reachable as the Decidable `WellPosed`/`Blocked` gate
  (or ship it as the documented reusable query), close the Anchored referential
  residual, and resume D57 (m2 probe → m3 generator → m4 cut).

## 4. Source-tree map

| File | Item 1 | Item 2 |
|---|---|---|
| `kernel/src/query/evaluate/pattern.rs` | **fix** (binding/join) | **add** (array match) |
| `kernel/src/query/evaluate/mod.rs` | inspect (fixpoint) | — |
| `kernel/src/query/evaluate/return_shape.rs` | inspect (projection) | — |
| `kernel/src/query/ast.rs` | confirm | **add** node |
| `kernel/src/query/parser.rs` | — | **add** grammar |
| `kernel/src/query/lexer.rs` | — | confirm tokens (`...`) |
| `kernel/src/query/type_check.rs` | — | **add** array-pattern typing |
| `kernel/src/query/stratify.rs` | confirm | confirm |
| `docs/guides/eigenql/05-pattern-matching.md`, `10-stratification.md` | update (join semantics) | document array patterns |
| `kernel/tests/` (+ `#[cfg(test)]`) | tests | tests |

## 5. Sequencing & risk

1. **Item 1 first** — it's the prerequisite and the smaller change (localized to
   `evaluate/pattern.rs`); fixing it likely makes the documented Ancestor idiom and
   several latent DEFINE-join queries correct, independent of arrays.
2. **Item 2** — the larger surface (lexer→parser→ast→evaluate→type_check), but
   mechanically self-contained.
3. **Item 3** — validation + resume.

**Risk notes.** Item 1 changes core join evaluation — needs a regression pass over
existing query tests (derived relations are used elsewhere). Item 2 must keep the
array-*pattern* (property position) distinct from the array-*literal* (expression
position) in the grammar. Rebuild the kernel image (`docker compose build kernel`)
before re-running the live Reachable validation, since the gate runs in-kernel.
