# 12. Appendix

## 13.1. Grammar reference (EBNF)

The authoritative grammar is [D2 §3](../../design/d2-eigenql-specification.md#3-parser-grammar-ebnf). This appendix summarizes it for quick reference and documents the Phase 11e.2 additions (qualified-name function calls).

```ebnf
(* Top-level *)
Program           ::= Definition* Query
Definition        ::= 'DEFINE' Identifier '(' Variable (',' Variable)* ')' 'FROM' DefineBody
Query             ::= MatchPart GroupBy? ReturnClause? OrderBy? Limit? Offset? Distinct?

(* MatchPart, restricted DEFINE form, and clauses *)
MatchPart         ::= (UsingClause | UsingInstitution)* Clause+ WhereClause?
DefineBody        ::= UsingClause* MatchClause WhereClause?     (* no FIBER, no USING INSTITUTION *)
UsingClause       ::= 'USING' StringLit (',' StringLit)*
UsingInstitution  ::= 'USING' 'INSTITUTION' StringLit 'AS' Identifier
Clause            ::= MatchClause | FiberClause
MatchClause       ::= 'MATCH' Pattern (',' Pattern)*
FiberClause       ::= 'FIBER' InstitutionRef ':' QueryClass
                      '{' (ParamBinding (',' ParamBinding)*)? '}'
                      'AS' Variable
WhereClause       ::= 'WHERE' Expression (',' Expression)*       (* comma-list, implicitly ANDed *)
GroupBy           ::= 'GROUP' 'BY' Expression (',' Expression)*
ReturnClause      ::= 'RETURN' ResultClasses '{' (ReturnItem (',' ReturnItem)*)? '}'
ResultClasses     ::= '[' ']'                                     (* untyped result *)
                    | '[' Name (',' Name)* ']'                    (* one or more classes *)
                    | Name                                        (* single class, no brackets *)
                    | (* empty — go straight to '{' *)
OrderBy           ::= 'ORDER' 'BY' OrderItem (',' OrderItem)*
Limit             ::= 'LIMIT' Integer
Offset            ::= 'OFFSET' Integer
Distinct          ::= 'DISTINCT'

(* Patterns *)
Pattern           ::= ('NOT')? ClassRef '(' Variable ')' '{' (PropertyPattern (',' PropertyPattern)*)? '}'
                    | ('NOT')? Variable '{' (PropertyPattern (',' PropertyPattern)*)? '}'
PropertyPattern   ::= Name ':' (Variable | Literal)

(* Names *)
Name              ::= Identifier | StringLit
ClassRef          ::= Identifier | StringLit
InstitutionRef    ::= Identifier | StringLit

(* FIBER param bindings *)
ParamBinding      ::= Name ':' Expression

(* Return items *)
ReturnItem        ::= Name ':' Expression
OrderItem         ::= Expression ('ASC' | 'DESC')?

(* Expressions, in precedence order low → high *)
Expression        ::= OrExpr
OrExpr            ::= AndExpr ('OR' AndExpr)*
AndExpr           ::= EqualityExpr ('AND' EqualityExpr)*
EqualityExpr      ::= RelationalExpr (('=' | '<>') RelationalExpr)*
RelationalExpr    ::= AdditiveExpr ((CompareOp | 'IN' | 'NOT' 'IN' | 'LIKE' | 'NOT' 'LIKE') AdditiveExpr)*
CompareOp         ::= '<' | '<=' | '>' | '>='
AdditiveExpr      ::= MultiplicativeExpr (('+' | '-' | '||') MultiplicativeExpr)*
MultiplicativeExpr::= PowerExpr (('*' | '/' | '%') PowerExpr)*
PowerExpr         ::= UnaryExpr ('**' UnaryExpr)*
UnaryExpr         ::= ('NOT' | '+' | '-') UnaryExpr
                    | 'NOT' 'EXISTS' '(' Variable ')'
                    | PrimaryExpr
PrimaryExpr       ::= Literal
                    | Variable
                    | DotPath
                    | ScalarFn '(' ArgList ')'
                    | AggregateFn '(' Expression ')'
                    | QualifiedName '(' ArgList ')'              (* Phase 11e.2 *)
                    | Identifier                                 (* bare shortname literal *)
                    | '[' ArgList ']'
                    | '(' Expression ')'
DotPath           ::= Variable ('.' Identifier)+
QualifiedName     ::= Identifier ':' Identifier
ArgList           ::= (Expression (',' Expression)*)?
```

**Notes on the form above**

- `MatchPart` allows any interleaving of `USING` and `USING INSTITUTION` clauses — they're not ordered into two phases. `DefineBody` is the restricted form used in `DEFINE` rules: it permits `USING` but neither `USING INSTITUTION` nor `FIBER` (parser enforces this; see [chapter 10](10-stratification.md)).
- `FIBER` uses `':'` between the institution reference and the query class, not `'.'`.
- `WHERE` accepts a comma-separated expression list, all implicitly ANDed.
- `ReturnClause`'s class spec has three forms: bracketed name list (possibly empty), a single bare name, or omitted entirely (braces directly after `RETURN`).
- `IN`, `NOT IN`, `LIKE`, and `NOT LIKE` accept any `AdditiveExpr` on the right — typically an array literal for `IN` and a string for `LIKE`, but a variable bound to a list/string is also valid.
- Equality and relational chains are written as `*` to match the parser, but consecutive non-associative comparisons are unusual; pre-formed chains like `?a = ?b = ?c` are valid by grammar, evaluated left-associatively.
- `**` is parsed left-associatively in the current implementation despite the precedence table marking it right-associative; this is a known quirk — use parentheses when stacking exponentiation.
- A bare `Identifier` in expression position evaluates to the identifier text as a string literal (used to pass shortnames as values, e.g., in `RETURN`).

**Phase 11e.2 addition**: the `QualifiedName '(' ArgList ')'` alternative in `PrimaryExpr`. Qualified-name function calls dispatch through the institution registry at evaluate time — see [chapter 9](09-institutions.md).

## 13.2. Keyword reference

### Structural keywords

`USING`, `INSTITUTION`, `AS`, `DEFINE`, `FROM`, `MATCH`, `FIBER`, `WHERE`, `RETURN`, `GROUP`, `BY`, `ORDER`, `ASC`, `DESC`, `DISTINCT`, `LIMIT`, `OFFSET`

### Operator keywords

`AND`, `OR`, `NOT`, `IN`, `LIKE`, `EXISTS`

### Built-in function keywords

`DATE`, `TIMESTAMP`, `REGEX`, `LENGTH`, `CONTAINS`, `CONCAT`, `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`

### Literal keywords

`true`, `false` (lowercase)

### Reserved identifiers

None beyond the keywords. Short names (for classes, properties, variables) can be any non-keyword identifier.

## 13.3. Built-in function reference

All case-sensitive UPPERCASE. Listed alphabetically.

| Function | Args | Returns | Behavior |
|---|---|---|---|
| `AVG(expr)` | Expression | Float | Average over a group. Requires `GROUP BY` context. |
| `CONCAT(a, b)` | Array, Array | Array | Element-wise array concatenation. For strings, use `\|\|`. |
| `CONTAINS(arr, v)` | Array, any | Boolean | Membership check via `values_equal`. |
| `COUNT(expr)` | Expression | Integer | Count of non-null values in a group. Requires `GROUP BY`. |
| `DATE(s)` | String | String | Validate ISO 8601 date (`YYYY-MM-DD`). Passes through on success. |
| `LENGTH(x)` | String or Array | Integer | Unicode char count (strings) or element count (arrays). |
| `MAX(expr)` | Expression | any | Maximum value in a group. Requires `GROUP BY`. |
| `MIN(expr)` | Expression | any | Minimum value in a group. Requires `GROUP BY`. |
| `REGEX(s)` | String | String | Validate regex syntax. Passes through on success. |
| `SUM(expr)` | numeric Expression | Integer or Float | Sum over a group. Integer if all inputs integer and sum exact. |
| `TIMESTAMP(s)` | String | String | Validate ISO 8601 datetime with timezone. Passes through on success. |

## 13.4. Operator precedence table

From tightest (evaluated first) to loosest:

| Level | Operators | Associativity |
|---|---|---|
| 1 | Primary: literals, variables, `(...)`, function calls, aggregates, `[...]`, dot-paths | — |
| 2 | `**` (power) | left (parser quirk; see note) |
| 3 | Unary `NOT`, `+`, `-`, `NOT EXISTS` | right |
| 4 | `*` `/` `%` | left |
| 5 | `+` `-` `\|\|` | left |
| 6 | `<` `<=` `>` `>=` `IN` `NOT IN` `LIKE` `NOT LIKE` | left |
| 7 | `=` `<>` | left |
| 8 | `AND` | left |
| 9 | `OR` | left |

Note: `**` is conventionally right-associative in mathematics (so `2**3**2 = 2**(3**2) = 512`), but the current parser folds it left (`(2**3)**2 = 64`). Always parenthesise stacked exponents to be explicit.

Use parentheses when in doubt: `(?a + ?b) * ?c` vs `?a + (?b * ?c)`.

## 13.5. Result-document IRIs

| Constant | IRI |
|---|---|
| ResultSet class | `urn:eigenius:query:ResultSet` |
| Row result_class property | `urn:eigenius:query:result_class` |
| Rows array property | `urn:eigenius:query:rows` |
| Row count property | `urn:eigenius:query:row_count` |
| Matched boolean property | `urn:eigenius:query:matched` |
| Generated base | `urn:eigenius:query:gen:<hash>` |
| Result set IRI | `urn:eigenius:query:gen:<hash>:result` |
| Row class IRI | `urn:eigenius:query:gen:<hash>:row_class` |
| Row property IRI | `urn:eigenius:query:gen:<hash>:row:<name>` |
| FIBER response IRI | `urn:eigenius:query:gen:<hash>:fiber:<clause>:<binding>` |

`<hash>` is 16 hex characters (first 8 bytes of SHA-256 over the query text).

## 13.6. Institution dispatch quick reference

The kernel maintains two derived structures over the layer chain:

- [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) — by-IRI lookup over `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism` declarations on the chain. Built by `InstitutionIndex::from_layer`. ESL and EigenQL share it for compile-time classification.
- [`InstitutionRuntime`](../../../kernel/src/institution/runtime.rs) — `BTreeMap<Iri, Box<dyn Institution>>` keyed by institution IRI. WASM-runtime institutions are auto-registered from chain scan via [`build_wasm_institution_runtime`](../../../kernel/src/capability/registration.rs); in-process / external runtimes are caller-registered.

EigenQL classifies a `qualified_call` IRI against the index:

| Index entry | EigenQL emits | Runtime call |
|---|---|---|
| `Decidable` `QueryClass` | `Exp::NativeDecide(Constraint::Institution { … }, Unit)` (returns Verdict; project with postfix `HOLDS`/`FAILS`/`UNDECIDABLE`) | `Institution::query(query_handler, synthetic_input, ctx)` |
| `OnDemand` `QueryClass` | only inside FIBER clauses | `Institution::query(query_handler, input, ctx)` |
| `Comorphism` | only inside FIBER param value coercion | `extract_typed → transformation Component → reify` four-step pipeline |
| Class / property / built-in / aggregate | various | no institution call |

See [chapter 9](09-institutions.md) for the full surface and [D14 §9](../../design/d14-institution-realisation.md) for the protocol.

## 13.7. Execution entry points

From [`kernel/src/query/mod.rs`](../../../kernel/src/query/mod.rs):

```rust
pub fn execute(
    program_str: &str,
    layer: &Layer,
) -> Result<Vec<Resource>, Vec<QueryError>>

pub fn execute_with(
    program_str: &str,
    layer: &Layer,
    runtime: FiberRuntime<'_>,
) -> Result<Vec<Resource>, Vec<QueryError>>
```

- `execute` — convenience wrapper that supplies a default empty `FiberRuntime`. No `FIBER`, no institution-dispatched function calls. Useful for CLI local mode and tests.
- `execute_with` — full pipeline. Required for FIBER clauses and for Decidable QueryClass calls in expressions.

`FiberRuntime` shape (D14):

```rust
pub struct FiberRuntime<'a> {
    pub index: Option<&'a InstitutionIndex>,
    pub runtime: Option<&'a InstitutionRuntime>,
    pub components: Option<&'a ComponentRegistry>,
    pub overlay: Option<&'a [(Iri, Resource)]>,
    pub ctx: Option<&'a ExecutionContext>,
}
```

- `index` + `runtime` are required for FIBER clauses and Decidable function calls.
- `components` is required when any FIBER param uses comorphism coercion (the four-step pipeline applies the comorphism's transformation Component).
- `ctx` is required for any institution-dispatched call.
- `overlay` is populated automatically — pass `None`.

## 13.8. Related documents

- [D2 EigenQL specification](../../design/d2-eigenql-specification.md) — authoritative grammar and semantics
- [D14 Institution Realisation](../../design/d14-institution-realisation.md) — institution-kernel interface (supersedes D10)
- [D1 Eigon serialization format](../../design/d1-eigon-serialization-format.md) — resource/value model
- [ESL user guide](../esl/README.md) — the other surface language

## 13.9. Source index

All source references in the guide, collected here for easy navigation:

**Query module**:
- [kernel/src/query/mod.rs](../../../kernel/src/query/mod.rs) — pipeline entry points
- [kernel/src/query/ast.rs](../../../kernel/src/query/ast.rs) — AST types
- [kernel/src/query/lexer.rs](../../../kernel/src/query/lexer.rs) — tokenizer
- [kernel/src/query/parser.rs](../../../kernel/src/query/parser.rs) — parser
- [kernel/src/query/stratify.rs](../../../kernel/src/query/stratify.rs) — stratification checker
- [kernel/src/query/type_check.rs](../../../kernel/src/query/type_check.rs) — type validation
- [kernel/src/query/evaluate.rs](../../../kernel/src/query/evaluate.rs) — evaluator (pattern match, fixpoint, aggregate)
- [kernel/src/query/functions.rs](../../../kernel/src/query/functions.rs) — built-in function dispatch and helpers
- [kernel/src/query/document.rs](../../../kernel/src/query/document.rs) — result-document shaping
- [kernel/src/query/error.rs](../../../kernel/src/query/error.rs) — `QueryError`

**Institution module** (D14):
- [kernel/src/institution/runtime.rs](../../../kernel/src/institution/runtime.rs) — `Institution` trait, `InstitutionRuntime`
- [kernel/src/institution/registry.rs](../../../kernel/src/institution/registry.rs) — `InstitutionIndex` (derived from chain scan)
- [kernel/src/institution/dispatch.rs](../../../kernel/src/institution/dispatch.rs) — `AutoOnLoad` dispatch
- [kernel/src/institution/error.rs](../../../kernel/src/institution/error.rs) — `InstitutionError`
- [kernel/src/capability/registration.rs](../../../kernel/src/capability/registration.rs) — chain-scan auto-registration of WASM institutions / components
- [kernel/src/capability/wasm_institution_d14.rs](../../../kernel/src/capability/wasm_institution_d14.rs) — host bridge to the `eigenius-institution-d14` WIT world

**Core / institution ontology**:
- [ontologies/core/core-ontology.json](../../../ontologies/core/core-ontology.json) — shipped definitions of `Class`, `Property`, `Verdict`, etc.
- [ontologies/institution/institution-ontology.json](../../../ontologies/institution/institution-ontology.json) — `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`

---

Return to **[README](README.md)**.
