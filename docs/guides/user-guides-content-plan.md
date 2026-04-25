# User-Guides Content Plan: ESL & EigenQL

## Overall framing

**Target audience**: developers writing against eigenius — both domain modelers authoring ontologies/programs and query authors retrieving data. Not kernel internals; not institution implementers (that's a separate doc).

**Existing design docs** already present (to cite, not duplicate):
- [D7 ESL Surface Syntax](../design/d7-esl-surface-syntax.md) — Overview, Design Principles, Namespace Aliases, Program Syntax, Ontology Syntax, Resource Syntax, Grammar EBNF, Error Reporting, Implementation Plan, Decisions
- [D2 EigenQL Specification](../design/d2-eigenql-specification.md) — Overview, Lexical Grammar, Parser Grammar EBNF, AST, Type Checking, Evaluation, Error Format, Examples, Decisions Log, Appendices A (Result Documents), B (FIBER clauses)

The guides **complement** D7/D2 — design docs are spec-first, the guides are task-first. Reference D7/D2 for the grammar appendix; derive everything else from the implementation.

**Artifact locations** to create:
- `docs/guides/esl-user-guide.md`
- `docs/guides/eigenql-user-guide.md`

**Shared supporting references** (link from both guides):
- Design: [D1 Eigon format](../design/d1-eigon-serialization-format.md), [D10 Institutions](../design/d10-grothendieck-institution-protocol.md), [D11 Codata](../design/d11-codata-streams.md), [D18 Ontology-as-types](../design/d18-ontology-as-types-resolution.md), [D19 Inductive+sized types](../design/d19-inductive-types.md)
- Core ontology: [ontologies/core/core-ontology.json](../../ontologies/core/core-ontology.json)
- Institution registry: [kernel/src/institution/mod.rs](../../kernel/src/institution/mod.rs)

---

# ESL User Guide — content plan

## 1. Introduction (~1 page)
**Cover**: What ESL is, the two-layer design (HCL-style declarations + ML-style expressions), where it sits (compiles to Eigon-JSON resources; resources drive both type-checking and runtime). Relationship to kernel type theory (expressions type-check against the Mini-TT kernel at [kernel/src/nbe/check.rs](../../kernel/src/nbe/check.rs); resources resolve to kernel types at [kernel/src/program/ground.rs](../../kernel/src/program/ground.rs)).

**Refs**: [D7 §1–2](../design/d7-esl-surface-syntax.md), [kernel/src/esl/mod.rs](../../kernel/src/esl/mod.rs)

## 2. Quick tour — 6 worked examples (~2 pages)
Each example: 10–30 lines of ESL source + 2–3 sentences of what it does. Write them so they all compile and type-check (cite test files where the same or similar sources are exercised).

1. **Class + property + resource** — minimal structural ontology. Ref: [esl/compile.rs tests](../../kernel/src/esl/compile.rs) `compile_class`, `compile_resource`.
2. **Program with `let` + function application** — simplest computation. Ref: `compile_simple_program`.
3. **Inductive type + match** — e.g. sized Nat, `succ({j < i}, Nat(j))` with a match arm. Ref: [D19 §10](../design/d19-inductive-types.md), [ground.rs sized_nat_with_bounded_binder_decodes_to_sized_pi](../../kernel/src/program/ground.rs).
4. **Codata type + corecord** — sized stream shape. Ref: [ground.rs self_referential_sized_stream_from_esl](../../kernel/src/program/ground.rs).
5. **Component call + trailing config block** — `CompleteText(input) { model = "foo" }`. Ref: `compile_component_shorthand`.
6. **Institution-dispatched decide predicate** — `cap:within_tolerance(delta, 0.1)` in a program body. Ref: [expr.rs esl_decide_predicate_compiles_and_decodes](../../kernel/src/program/expr.rs).

## 3. Lexical structure (~2 pages)
**Cover**: tokens, keywords, identifiers (bare + qualified `ns:local`), literals (string with escapes, int with sign, float, bool), operators/punctuation, comments (`//`, `/* */`), Unicode λ for lambda. Include a complete keyword list.

**Refs** (token source of truth):
- [kernel/src/esl/lexer.rs](../../kernel/src/esl/lexer.rs) `TokenKind` enum: top-level keywords (`namespace`, `class`, `property`, `resource`, `program`, `codata`, `data`), expression keywords (`let`, `case`, `match`, `returning`, `Construct`, `map`, `reduce`, `corecord`), operators (`=`, `->`, `\`, `λ`, `.`, `;`, `:`, `,`, `<` for size bounds), braces.
- [D7 §7 grammar](../design/d7-esl-surface-syntax.md) for a reference EBNF.

## 4. Declarations (~6–8 pages)
For each declaration form: syntax, semantics, emitted resource shape, relevant ontology class. Explain every field admitted.

### 4.1 `namespace`
Alias → URI mapping, scope-local.
**Refs**: [parser.rs parse_namespace](../../kernel/src/esl/parser.rs), D7 §3.

### 4.2 `class`
`requires` (direct Sigma components), `recommends` (Option-wrapped), `allows_only`, `class_types`, inheritance via `extends`.
**Refs**: [compile.rs compile_class](../../kernel/src/esl/compile.rs), [ground.rs collect_properties](../../kernel/src/program/ground.rs), D18.

### 4.3 `property`
Data-type-bearing property with scalar constraints (`min_value`, `max_value`, `min_length`, `max_length`, `pattern`, `format`) and the institution-dispatched **decide predicate** annotation from Phase 11c.
**Refs**: [compile.rs compile_property](../../kernel/src/esl/compile.rs), [ground.rs resolve_property_type](../../kernel/src/program/ground.rs), [term.rs Constraint enum](../../kernel/src/nbe/term.rs), [institution/mod.rs FiberReasoner::decide](../../kernel/src/institution/mod.rs).

### 4.4 `resource`
Typed resource construction; constraint evaluation at load time.
**Refs**: [compile.rs compile_resource](../../kernel/src/esl/compile.rs).

### 4.5 `data` — inductive types
Parameter telescope (including `Size` kind per Phase 11h); constructors with positional args and **brace-delimited bounded binders** (`{j < i}`, `{j : core:Size}`, `{j : core:Size < i}`); self-references; positivity requirement.
**Refs**: [parser.rs parse_ctor_arg](../../kernel/src/esl/parser.rs), [ast.rs CtorArg enum](../../kernel/src/esl/ast.rs), [compile.rs compile_ctor_binder](../../kernel/src/esl/compile.rs), [ground.rs decode_arg_type + decode_ctor_arg](../../kernel/src/program/ground.rs), [positivity.rs](../../kernel/src/nbe/positivity.rs), D19 §2–7 + §8.

### 4.6 `codata` — coinductive types
Parameter telescope; observation types via the **TypeExpr** sublanguage (function arrows, size-bound arrows, parameterised refs); self-references via `Exp::CodataType`.
**Refs**: [parser.rs parse_codata + parse_type_expr + parse_type_atom](../../kernel/src/esl/parser.rs), [ast.rs TypeExpr enum](../../kernel/src/esl/ast.rs), [compile.rs compile_codata + compile_type_expr](../../kernel/src/esl/compile.rs), [ground.rs resolve_codata_type + decode_codata_observation_type](../../kernel/src/program/ground.rs), D11, D19 §8.

### 4.7 `program`
Input type, output type, body expression; compiles to a `Lam` wrapped by a `Pi`. Body can use any Expr form (see §5).
**Refs**: [compile.rs compile_program](../../kernel/src/esl/compile.rs), [program/expr.rs parse_program](../../kernel/src/program/expr.rs).

## 5. Expressions — per-construct reference (~10–12 pages)
The core of the guide. For each `Expr` variant:
- **Syntax** (with grammar fragment)
- **Kernel `Exp` it compiles to** (cite the eval arm)
- **Type-check rule** (check.rs arm): what types are expected, what's inferred
- **Evaluation rule** (eval.rs arm)
- **Capability mode notes** — if behavior differs between Pure/Read/IO/Check

Go through: `Let`, `Apply` (+ institution-dispatch variants), `Lambda`, `Case`, `Match`, `Construct`, `Project` (property access / observation), `MapExpr`, `ReduceExpr`, `CoRecord`, `Pair`, `Literal`, `Var`.

**Refs** per construct:
- [ast.rs Expr enum](../../kernel/src/esl/ast.rs)
- Compile: [esl/compile.rs compile_expr](../../kernel/src/esl/compile.rs)
- Decode: [program/expr.rs parse_expression + sub-parsers](../../kernel/src/program/expr.rs)
- Kernel check arms: [nbe/check.rs](../../kernel/src/nbe/check.rs)
- Kernel eval arms: [nbe/eval.rs](../../kernel/src/nbe/eval.rs)

Special sub-sections:
- **Constructor application** `Ctor(args)` → [`Exp::InductiveCtor`](../../kernel/src/nbe/term.rs), with bounded-size-arg semantics (Phase 11g → check_inductive_ctor_args)
- **Pattern match** `match e returning T { arm => body; ... }` — motive inference + sized-induction hypothesis insertion in arm scope
- **Corecord** `corecord { head = e; tail = λj. ... }` — `Lam` vs `SizedPi` check arm (Phase 11f productivity-by-typing)
- **Institution capability call** `urn:ins:cap(args)` — classification at compile → `Exp::NativeDecide(Institution{..}, Unit)` or `Exp::InstitutionInvoke{..}` (Phase 11e.1)

## 6. Type theory primer (~4–6 pages)
Keep brief; this is a user guide, not a textbook. Cover:
- **Universes**: `Set` and `Type(n)`, cumulativity, why it matters in practice
- **Π-types** (dependent functions) and **Σ-types** (dependent pairs) — how they underlie classes and programs
- **Inductive types**: constructors, recursor/match, iota reduction
- **Coinductive types**: observations, corecord productivity
- **Sized types**: size variables, `∞`, bounded binders, termination/productivity proofs — link to the sized-Nat and sized-stream examples
- **Identity types**: `Refl`, J eliminator, use in constraint witnesses
- **Normalization-by-evaluation**: why it's the conversion engine, what "neutral terms" are

**Refs**: [nbe/term.rs](../../kernel/src/nbe/term.rs) and [nbe/val.rs](../../kernel/src/nbe/val.rs) for the AST shape; [D19 §3](../design/d19-inductive-types.md) for the core theory; [D11](../design/d11-codata-streams.md) for codata.

## 7. Capability modes (~1.5 pages)
**Four modes** from [EvalCtx](../../kernel/src/nbe/eval.rs):
- **Pure** — normalization only, no external access. Type-checking's default.
- **Read** — adds `Arc<Layer>` for property / class resolution.
- **IO** — adds component registry, institution registry, trace store, optional task context. Runtime.
- **Check** — Pure + institution registry. Type-checker's mode when institution-dispatched constraints need to fire during check.

Tabulate which kernel AST nodes have different behavior in each mode (e.g., `App` on a component dispatches in IO, returns neutral in Pure; `NativeDecide(Institution{..}, _)` decides in Check/IO, stays neutral in Pure/Read).

## 8. Institutions in ESL (~2 pages)
**Cover**:
- What an institution is (fiber reasoner + registered capabilities)
- Declaration surface via the **core ontology**: `urn:eigenius:institution:Comorphism` class from Phase 11d
- What institutions declare: `morphism_types`, `query_types`, `structural_properties`, `comorphism_types`, `decide_procedures`
- How ESL code invokes capabilities (from 11e.1): `cap:predicate(args)` → decide, `cap:translate(source)` → comorphism
- Default behaviors (`DecResult::Undecidable`, `InstitutionError::UnknownType` for un-overridden translate)

**Refs**: [D10](../design/d10-grothendieck-institution-protocol.md), [institution/mod.rs FiberReasoner trait](../../kernel/src/institution/mod.rs), [nbe/check.rs NativeDecide check arm](../../kernel/src/nbe/check.rs), [nbe/eval.rs decide_constraint + InstitutionInvoke eval arm](../../kernel/src/nbe/eval.rs).

Include the [life-science §16.3](../design/life-science-requirements.md) motivating example (RMSD predicate, docking→assay comorphism) written in ESL.

## 9. Error messages (~1 page)
Samples from tests: `"bare name 'X' has no namespace"`, `"cannot infer type of ..."`, `"InductiveCtor 'X.succ': size argument ... is not strictly below upper bound ..."`, `"comorphism 'X' expects exactly 1 source argument, got N"`. Explain each and how to fix.

## 10. Appendix: EBNF + keyword reference
Link to [D7 §7](../design/d7-esl-surface-syntax.md) as authoritative. Include only delta (bounded-binder syntax added in Phase 11h, institution-capability syntax from 11e.1).

---

# EigenQL User Guide — content plan

## 1. Introduction (~1 page)
**Cover**: What EigenQL is (read-only query over the layered knowledge graph), relation to ESL (ESL computes; EigenQL retrieves + filters; they share the same kernel primitives for institution dispatch), compile pipeline (lex → parse → stratify → type-check → evaluate → document-wrap).

**Refs**: [D2 §1](../design/d2-eigenql-specification.md), [kernel/src/query/mod.rs](../../kernel/src/query/mod.rs).

## 2. Quick tour — 7 worked examples (~2 pages)
1. **Basic MATCH + RETURN** — find all classes.
2. **Property filter** — MATCH with a property pattern.
3. **DEFINE** — derived relation (transitive closure).
4. **WHERE** — boolean condition.
5. **GROUP BY + aggregate** — count per class.
6. **FIBER clause** — institution-dispatched query (D10 §6, D2 §3.5/§5.8/§6.12).
7. **Institution decide predicate in WHERE** — `cap:within_tolerance(d, 0.1)` filter (Phase 11e.2).

Refs: [query/evaluate.rs](../../kernel/src/query/evaluate.rs) tests (find_all_classes, parser_accepts_qualified_function_calls, etc).

## 3. Lexical structure (~1.5 pages)
Tokens, keywords, identifiers, qualified names (`ns:local`), literals (string/int/float/bool), operators/punctuation, `?variable` syntax (which has no ESL analogue), string literals as `Name::FullIri`.

**Refs**: [kernel/src/query/lexer.rs](../../kernel/src/query/lexer.rs) `TokenKind`, [D2 §2](../design/d2-eigenql-specification.md).

## 4. Program structure (~3 pages)
The top-level Query shape (Program { definitions, query: Query { body, group_by, result_classes, result, order_by, limit, offset, distinct }).

Clause-by-clause:
### 4.1 `USING <class-iri>` — class imports
### 4.2 `USING INSTITUTION alias = <iri>` — fiber aliases
### 4.3 `DEFINE <name>(?x, ?y) FROM <body>` — derived relations + stratification
### 4.4 `MATCH <subject> [(class)] { props }` — main pattern-match block
### 4.5 `WHERE <expr>` — boolean filter, including decide-predicate invocations
### 4.6 `FIBER <institution>:<query-class> { ... } AS ?var` — institution dispatch
### 4.7 `RETURN [classes] { prop: expr, ... }` — result shape (D2 Appendix A)
### 4.8 `GROUP BY <expr>` — aggregation key
### 4.9 `ORDER BY / LIMIT / OFFSET / DISTINCT`

**Refs**: [ast.rs Program + Query + MatchPart + Clause + Pattern + FiberClause](../../kernel/src/query/ast.rs), [parser.rs](../../kernel/src/query/parser.rs), [D2 §3](../design/d2-eigenql-specification.md).

## 5. Pattern matching (~2 pages)
- **Subjects**: variables (`?x`) and literal IRIs
- **Class predicates**: `(ClassName)` after the subject
- **Property patterns**: `{ prop: value | ?var | [list] }`
- **Negation**: `NOT { ... }` — semantics under stratification
- **Variable binding**: how bindings accumulate across clauses

**Refs**: [evaluate.rs apply_pattern + apply_negated_pattern](../../kernel/src/query/evaluate.rs), [D2 §4 + §6](../design/d2-eigenql-specification.md).

## 6. Expressions — per-construct reference (~4–5 pages)
Every `Expression` variant ([ast.rs](../../kernel/src/query/ast.rs)):
- **Literal** — string, int, float, bool
- **Variable** — binding lookup
- **Binary** — 11 ops (compare, logical, arith)
- **Unary** — 3 ops (not, neg)
- **NotExists(?var)** — anti-join
- **FunctionCall { name, args }** — three dispatch paths:
  1. Built-in keyword functions (`DATE`, `LENGTH`, `CONCAT`, `REGEX`, `CONTAINS`, `TIMESTAMP`)
  2. **Institution-dispatched decide predicate** (Phase 11e.2) — `ns:predicate(args)` → `reasoner.decide(..)` → `Value::Boolean(Holds)`
  3. **Institution-dispatched comorphism** (Phase 11e.2) — `ns:translate(src)` → `reasoner.translate(..)` → `Value::Embedded(resource)`
- **Aggregate** — COUNT/SUM/AVG/MIN/MAX
- **DotPath** — resource property chain walk
- **Array / Object** — result-shape constructors

For each: evaluation rule + example.

**Refs**: [evaluate.rs eval_expression + dispatch_institution_call + eval_aggregate](../../kernel/src/query/evaluate.rs), [functions.rs call_function](../../kernel/src/query/functions.rs).

## 7. FIBER clauses (~2 pages)
How queries dispatch to institutions. Covers:
- `USING INSTITUTION alias = "urn:..."`
- `FIBER alias:QueryClass { param: value } AS ?var`
- The transient overlay model (D2 §6.12) — FIBER results scoped to the current query
- Required `FiberRuntime` (institution registry + execution context) — without it, FIBER clauses error

**Refs**: [evaluate.rs apply_fiber_clause](../../kernel/src/query/evaluate.rs), [D2 §3.5/§5.8/§6.12](../design/d2-eigenql-specification.md), [D10 §5](../design/d10-grothendieck-institution-protocol.md).

## 8. Institutions in EigenQL (~2 pages)
Parallel to the ESL guide's §8 but from the query perspective:
- How to register an institution that EigenQL can query
- The three entry points into an institution from a query:
  1. FIBER clause → `reasoner.query(..)` (pre-existing, from D10)
  2. Decide predicate in WHERE / RETURN → `reasoner.decide(..)` (Phase 11e.2)
  3. Comorphism call in expression → `reasoner.translate(..)` (Phase 11e.2)
- How classification works at evaluate time ([InstitutionRegistry::classify](../../kernel/src/institution/mod.rs))
- Life-science example: `SELECT ... WHERE docking:within_rmsd(p1, p2, 2.0)` filters structural comparisons by an institution-decided predicate

**Refs**: [evaluate.rs dispatch_institution_call](../../kernel/src/query/evaluate.rs), [institution/mod.rs classify](../../kernel/src/institution/mod.rs), [D10](../design/d10-grothendieck-institution-protocol.md).

## 9. Stratification (~1.5 pages)
Why it exists; how negation + DEFINE recursion coexist (semantics from D2 §5); what queries get rejected and the error messages that surface.

**Refs**: [query/stratify.rs](../../kernel/src/query/stratify.rs), [D2 §5 Type Checking Rules](../design/d2-eigenql-specification.md).

## 10. Result format (~1 page)
D2 Appendix A — how RETURN shapes become Resources; Property / Class metadata synthesis; how to consume results programmatically.

**Refs**: [query/document.rs](../../kernel/src/query/document.rs), [D2 Appendix A](../design/d2-eigenql-specification.md).

## 11. Error messages (~1 page)
Stratification errors, type-check errors, runtime errors. Concrete samples with fixes.

## 12. Appendix: EBNF + keyword + built-in function reference
Link to [D2 §3](../design/d2-eigenql-specification.md) for authoritative grammar. Include the Phase 11e.2 additions (qualified-name function calls).

---

## Shared appendix (common to both guides)

A short section explaining the **institution capability classification** table:

| IRI resolves to | ESL emits | EigenQL emits | Kernel behavior |
|---|---|---|---|
| Component | `Exp::App(Var(iri), ..)` | `functions::call_function` fallthrough | Component dispatch via `dispatch_component` in IO mode |
| Registered Comorphism | `Exp::InstitutionInvoke { iri, source }` | `dispatch_institution_call(Comorphism, ..)` | `reasoner.translate` |
| Registered DecidePredicate | `Exp::NativeDecide(Constraint::Institution{..}, Unit)` | `dispatch_institution_call(DecidePredicate, ..)` | `reasoner.decide` |
| Inductive Constructor | `Exp::InductiveCtor(decl, ctor, args)` | n/a (not used in queries) | `Val::InductiveVal` |
| Class IRI in type position | `Exp::EigonClass(iri)` | resolved via layer | `find_sigma_field` |
| Unknown | compile error ("unknown namespace" etc.) | `call_function` "no such function" | error |

This table is the user-facing summary of Phase 11e's design decision to unify everything through IRI classification. Link to both guides' institutions sections.

---

## Execution notes

**Order to write**: EigenQL first (smaller surface, less type theory to explain), then ESL (builds on the institution-capability table already drafted).

**Estimated length**:
- ESL guide: ~35–45 pages
- EigenQL guide: ~20–25 pages
- Shared appendix: ~3 pages

**Sample code**: every example should be copy-pasted from — or at minimum compilable against — the test suite. Citing the test file that exercises a shape grounds the example in actually-working code.

**Derivation discipline**: every claim about behavior should link to the source file + line range. If the plan says "type-check does X for construct Y," the prose should cite the arm.

**Not covered by these guides** (separate docs):
- Institution *implementation* guide (for developers writing new reasoners) — different audience
- Phase 0–10 feature deep-dives already in existing design docs
- Ops/deployment (Phase 13 material)
