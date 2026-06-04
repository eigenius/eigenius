# D45 — `BIND` Clause: Body-Scoped Variable Introduction

*Status: **withdrawn** · superseded by the D43 surface reset (June 2026).*

*Companion documents: [D2 EigenQL specification](d2-eigenql-specification.md), [D43 text and vector retrieval](d43-text-and-vector-retrieval.md).*

---

## Withdrawal note

D45 was proposed to close a gap exposed by the D35 §7.4 / D43 §6.5 worked example: that example used SQL-style `AS` bindings (`RRF(text_score, vec_score)` referencing RETURN-renamed columns), which EigenQL doesn't support. BIND would have introduced SPARQL-style `BIND(expr AS ?var)` so per-row score variables (`?ts`, `?vs`) could be named and reused.

That motivation was retrieval-specific, and it has been **eliminated by the D43 surface reset**. D43 now exposes retrieval through a single `~` operator (D43 §3.3) with hidden mechanism: per-row scores aren't user-visible, RRF isn't a user-visible function, fusion happens internally, and there is nothing left for a BIND-introduced variable to name. The §6.5 worked example reduces to two `~` operators in a `WHERE`-`OR` plus a plain `TOP K`; no intermediate score bindings are needed at any point.

With its anchoring use case gone, BIND has no remaining justification in v1. The general "name an expression once for DRY" motivation is real but weak — none of EigenQL's current workloads benefit measurably, users can just repeat short expressions, and keeping an unused surface in the language taxes documentation, tests, and reader attention.

**Status of the design.** Withdrawn. If a future workload reveals a strong need for query-body variable introduction (Datalog-style `LET`, SPARQL-style `BIND`, or another shape), the design can be revisited then — but the operational answers will likely differ from this document because the constraints will differ. The §1–§12 below remain as historical context only and should not be treated as a roadmap.

**Implementation consequence.** If any BIND surface had been implemented (it briefly was, prior to the D43 reset), it is removed in the same pass that lands the `~` operator. See `d43-implementation-plan.md`'s M7 milestone for the consolidated language-surface delta.

---

## 1. Motivation

> *Historical — superseded by the D43 §3 surface reset. Retained for context only.*


EigenQL today introduces query variables in exactly one place: MATCH patterns. RETURN renames an expression into a result column but does not produce a referenceable identifier; ORDER BY / TOP K BY consequently can reference *MATCH-bound* variables and (by a naming kludge) *RETURN-renamed-with-the-same-short-name* row columns, but not RETURN-introduced expression aliases.

This gap surfaces sharply for the hybrid-retrieval pipeline in [D43 §6.5](d43-text-and-vector-retrieval.md). The worked example assumes a SQL-style `AS` binding:

```eigenql
RETURN ?d,
       TEXT_SCORE(?desc, "...") AS text_score,
       VECTOR_SIM(?e, ...)      AS vec_score,
       RRF(text_score, vec_score) AS fused
TOP 20 BY fused DESC
```

In EigenQL today, `text_score`, `vec_score`, and `fused` are not referenceable from the RRF arguments or from TOP K BY. The user must inline the source expressions repeatedly. M7.3 (hybrid retrieval) already lands this workaround in tests; the verbosity is real but tolerable. The structural fix — introducing variable bindings in the query body — is the subject of this document.

## 2. The decision in one sentence

EigenQL adopts SPARQL 1.1's `BIND(expr AS ?var)` construct verbatim in syntax and semantics. The choice is made because SPARQL has already worked the design questions out, the construct fits EigenQL's per-binding WHERE evaluator without restructuring, and EigenQL's grammar inherits no surprises from established RDF-query practice.

## 3. Scope

In scope:

- A new `BIND(expr AS ?var)` clause that may appear in MATCH or in WHERE position (the two clause-bodies an EigenQL query has). Introduces `?var` into the binding scope for all clauses to its right.
- Typecheck rules: `?var` must be fresh; `expr` must be evaluable against the variables already in scope at the BIND's position; the bound variable participates in the existing variable-binding analysis for RETURN / ORDER BY / TOP K BY / GROUP BY.
- Evaluation: per-binding evaluation of `expr` against the current binding, with the result added to the binding under `?var`.
- Composition with the existing retrieval primitives (TEXT_SCORE, VECTOR_SIM, EMBED) and with arithmetic expressions.

Out of scope:

- **Re-binding.** A BIND whose `?var` is already in scope is a typecheck error, matching SPARQL.
- **Non-monotone bindings.** RRF inside BIND is rejected at typecheck for v1. RRF stays a planner-recognised special form in RETURN and TOP K BY position, as already implemented in M7.2. Lifting this restriction is contingent on the relation-wide-stratum work sketched in §9.
- **Aggregates inside BIND.** Aggregates inside BIND would have the same non-monotone shape as RRF; same rejection.
- **BIND inside disjunctions.** A `WHERE { ... BIND(...) ... OR ... }` does not propagate the bound variable to the OR's other branch. v1 rejects BIND inside an OR-disjoined expression; the typecheck is structural ("BIND must be a top-level WHERE conjunct").
- **A separate LET keyword.** The `BIND(expr AS ?var)` spelling is unambiguous and inherits SPARQL idiom; introducing both BIND and LET would be needless duplication.

## 4. Surface syntax

```
WhereItem ::=
    expression                       -- existing filter form
  | "BIND" "(" expression "AS" Variable ")"

MatchPart ::= ... ( WhereItem ("," WhereItem)* )? ...
```

The `BIND` keyword is reserved. `AS` already exists as a SPARQL-derived keyword reuse; EigenQL adds it solely for this construct (no other AS-uses are introduced — RETURN's `name: expr` shape is unchanged).

The grammar position is the same as a WHERE filter: BIND clauses appear comma-separated alongside filter expressions. Order is significant — a BIND introduces a variable visible to subsequent clauses in left-to-right textual order, including subsequent BINDs.

## 5. Semantics

**Evaluation model.** A WHERE list of clauses `c1, c2, ..., cn` evaluates left-to-right. The set of bound variables grows monotonically: each BIND adds a variable; each filter constrains the binding set but adds no variables.

For a candidate binding `β` reaching `BIND(expr AS ?var)`:
1. Evaluate `expr` against `β`. If evaluation errors (e.g. division by zero, EMBED dispatch failure), the binding is dropped — same semantics as a filter that raises.
2. Bind `?var` to the result. The augmented binding `β' = β ∪ {?var → result}` is fed to the next clause.

If `expr` produces an `Unbound` / `Nothing` value (e.g. a TEXT_SCORE for a row whose text is non-string), the BIND is treated as a filter that fails: the candidate binding is dropped. This matches SPARQL's "error in BIND drops the solution" rule.

**Static guarantees.**

- A BIND never adds rows. The candidate-binding set after a BIND is a subset of the set before.
- A BIND can only introduce variables that are not yet in scope (typecheck-enforced).
- BIND expressions are pure with respect to the chain (no side effects); they may dispatch IO components (EMBED) which the existing pre-pass caching covers.

**Visibility.** A variable bound by `BIND(expr AS ?var)` is visible to:

- Every clause to the right of the BIND within the same MatchPart's WHERE list.
- The RETURN clause.
- The ORDER BY / TOP K BY clauses.
- The GROUP BY clause.

Not visible to:

- Clauses textually to the left of the BIND.
- Any DEFINE rule's body (rules are scoped to their own bindings).
- FIBER clauses that precede the BIND.

## 6. Typecheck rules

Three new error categories:

| Error | Triggered by |
|---|---|
| `bind_redefines_variable` | `BIND(expr AS ?var)` where `?var` is already bound by a MATCH pattern, a prior BIND, or a FIBER `AS ?bound`. |
| `bind_expr_uses_unbound_var` | `BIND(expr AS ?var)` where `expr` references a `?other` not yet in scope at the BIND's position. |
| `bind_rrf_not_supported` | `BIND(RRF(...) AS ?var)`. v1 rejection per §3; revisit when relation-wide bindings ship. |

The existing `unbound_variable` check now runs against the *accumulating* bound-variable set as it walks WHERE clauses left to right, rather than the all-MATCH-bound set. This is a small but real change to `check_expression_variables`: instead of a single `BTreeSet<String>` computed once, it becomes an accumulator that grows past each BIND.

DEFINE-rule WHERE clauses receive the same treatment.

## 7. Evaluation strategy

The current evaluator pattern (in [`evaluate/pattern.rs`](../../kernel/src/query/evaluate/pattern.rs) and [`evaluate/mod.rs`](../../kernel/src/query/evaluate/mod.rs)) evaluates WHERE conditions as a single retain pass after MATCH produces candidate bindings. Under BIND, the WHERE list becomes a sequence of binding-transformations followed by filter-retains, applied left to right:

```
bindings = apply_match_patterns(...)
for clause in where_list:
    match clause:
        Filter(e):  bindings.retain(|b| eval(e, b).is_truthy())
        Bind(e, v): bindings = bindings.into_iter()
                              .filter_map(|b| eval(e, b).ok()
                                  .map(|val| { b.insert(v, val); b }))
                              .collect()
```

The structural change is one new clause type in the WHERE evaluator's loop and one extension to the AST. No new infrastructure is required.

The existing RRF pre-pass machinery (M7.2) continues to run after WHERE evaluation and before RETURN shaping, walking the final binding set for `Expression::Rrf` nodes in RETURN / TOP K BY. BIND variables are first-class bindings by that point — the pre-pass reads them like any other variable.

## 8. Worked examples

**D43 §6.5 hybrid pipeline:**

```eigenql
MATCH Doc(?d) { description: ?desc, embedding: ?e }
WHERE BIND(TEXT_SCORE(?desc, "wal truncation concurrent commit") AS ?ts),
      BIND(VECTOR_SIM(?e, EMBED("rolling back a partial commit")) AS ?vs),
      TEXT_MATCH(?desc, "wal truncation concurrent commit")
        OR VECTOR_NEAR(?e, EMBED("rolling back a partial commit"), k: 100)
RETURN [] {
    d: ?d,
    text_score: ?ts,
    vec_score:  ?vs,
    fused:      RRF(?ts, ?vs)
}
TOP 20 BY ?fused DESC
```

Every `?` is a single-use variable. RRF receives bound variables (typecheck-recognised score expressions because they're variables bound to TEXT_SCORE / VECTOR_SIM results — the existing recogniser composes through the chain). TOP K BY references a RETURN-renamed-same-short-name variable; that's the same kludge ORDER BY uses today (separate from this work, see §9).

**Arithmetic combinations:**

```eigenql
MATCH Doc(?d) { description: ?desc, length: ?len }
WHERE BIND(TEXT_SCORE(?desc, "foo") AS ?raw),
      BIND(?raw / LOG(?len + 1) AS ?normalized)
RETURN [] { d: ?d, score: ?normalized }
TOP 10 BY ?score DESC
```

Length-normalised text scoring. The second BIND uses the first BIND's variable; left-to-right evaluation makes this work.

## 9. Interaction with the rest of EigenQL

**RRF stays where it is.** Until non-monotone BIND lands, RRF remains parser-recognised in RETURN and TOP K BY (D43 §3.6 / M7.2), not in BIND.

**TOP K BY `?fused` and the sort-restructure question.** Even with BIND providing `?fused`, the sort path (`return_shape::sort_results`) operates on shaped result resources, not on bindings. `?fused` is read from the row's `fused` short-name property. The existing kludge — variable name must match RETURN short-name — applies here too. Lifting the kludge is M7.4's work: restructure `sort_results` to evaluate the sort expression against the underlying binding rather than property-name lookup. That work is *independent* of D45 and lands on its own schedule.

**FIBER `AS ?bound` is unchanged.** FIBER's `AS ?bound` already produces a variable visible to the rest of the query; BIND adds another path to variable introduction without disturbing FIBER.

**Rule bodies (DEFINE).** A DEFINE rule's WHERE may use BIND. The rule's head variables continue to project a subset of MATCH-bound variables; whether BIND-introduced variables can appear in rule heads is left to a follow-up (it's a Datalog-style extension worth its own discussion). v1: rule heads project MATCH-bound variables only.

## 10. Prior art

- **SPARQL 1.1 `BIND`**. The construct EigenQL inherits. SPARQL specifies `BIND(?expr AS ?var)` with row-local semantics; this document mirrors it.
- **SQL `AS`**. Comparable user-visible effect but different mechanism: SQL `AS` introduces a SELECT-list alias scoped to the same SELECT (and ORDER BY, dialect-dependent). EigenQL deliberately does *not* adopt this; binding-introduction is a body construct, not a projection construct.
- **Datalog/Prolog conjunction**. Variables appear in body literals; first occurrence binds, subsequent occurrences unify. SPARQL's BIND is the constrained form: no unification, no re-binding. EigenQL inherits the constraints.
- **D43 §3.6 RRF**. The motivating use case; D45 closes the surface gap exposed there.

## 11. Implementation plan

Five steps, each small and individually testable:

1. **AST**: add `Expression::Bind { expr: Box<Expression>, var: Variable }` or a separate `WhereItem` enum, and an AST predicate (`is_bind`) so the typecheck / evaluator can branch cleanly.
2. **Parser**: extend WHERE-clause parsing to recognise `BIND(expr AS ?var)`. Add `Bind` and `As` token kinds (`AS` is already a token from FIBER's `INSTITUTION ... AS alias` reuse — confirm and reuse).
3. **Typecheck**: refactor `check_expression_variables` from a single bound-vars set to an accumulating walker. Emit the three new error categories from §6. Reject `BIND(RRF(...) AS ?var)`.
4. **Evaluator**: extend the WHERE evaluation loop in `evaluate/pattern.rs` (or wherever WHERE conditions are currently consumed) to handle the new clause shape. Each BIND becomes a `filter_map` over the binding set.
5. **Tests**: a parser test per surface form; a typecheck test per error category; an end-to-end test that mirrors §8's hybrid pipeline (without TOP K BY of RRF, which awaits M7.4) and confirms the binding is visible to RETURN.

Estimated scope: one session for the language change plumbing; one session for the full test sweep. Independent of and orthogonal to M7.4.

## 12. Future work

- **Relation-wide BIND** (RRF / aggregates in BIND position). Requires a stratification pass that separates row-local clauses from relation-wide ones, similar to the existing DEFINE-rule stratification. Lands when ranking-and-fusion patterns multiply enough to warrant the structural change.
- **BIND-introduced variables in rule heads**. Would let a DEFINE rule export a computed value, not just MATCH bindings. Datalog-flavoured extension worth its own design pass.
- **`AS` for RETURN columns**. Today's RETURN syntax `{ name: expr }` already gives column naming; whether to alias `AS` into RETURN for SQL-familiarity is a separable cosmetic question and out of scope for D45.
