# 12. Appendix

## 13.1. Grammar reference (EBNF)

The grammar below is read off the parser — [`kernel/src/query/lexer.rs`](../../../kernel/src/query/lexer.rs) and [`kernel/src/query/parser.rs`](../../../kernel/src/query/parser.rs) — and is current as of 2026-08-20. [D2 §3](../../design/d2-eigenql-specification.md#3-parser-grammar-ebnf) is the design document for the base language, but it predates two extensions the parser has shipped since: [D43](../../design/d43-text-and-vector-retrieval.md) (the similarity operator `~`, its hint set, and `TOP`) and [D59](../../design/d59-eigenql-array-patterns-and-derived-joins.md) (array element patterns). `USING NAMESPACE` and `FIBER … INTO` are likewise parser surface not covered by D2 §3. Where a document and the parser disagree, the parser is what runs.

`X?` is optional, `X*` is zero or more, `X+` is one or more.

```ebnf
(* Top-level *)
Program           ::= Definition* Query
Definition        ::= 'DEFINE' Identifier '(' Variable (',' Variable)* ')' 'FROM' DefineBody
Query             ::= MatchPart GroupBy? ReturnClause? OrderBy?
                      Limit? Top? Offset? Distinct?
                      (* positional and once each, then end of input *)

(* MatchPart, restricted DEFINE form, and clauses *)
MatchPart         ::= UsingDecl* Clause+ WhereClause?
DefineBody        ::= UsingDecl* MatchClause WhereClause?  (* no FIBER, no USING INSTITUTION *)
UsingDecl         ::= 'USING' StringLit (',' StringLit)*            (* class imports *)
                    | 'USING' 'NAMESPACE' StringLit (',' StringLit)*  (* short-name scope *)
                    | 'USING' 'INSTITUTION' StringLit 'AS' Identifier
Clause            ::= MatchClause | FiberClause
MatchClause       ::= 'MATCH' Pattern (',' Pattern)*
FiberClause       ::= 'FIBER' InstitutionRef ':' QueryClass
                      '{' (ParamBinding (',' ParamBinding)* ','?)? '}'
                      'AS' Variable ('INTO' StringLit)?
WhereClause       ::= 'WHERE' Expression (',' Expression)*       (* comma-list, implicitly ANDed *)
GroupBy           ::= 'GROUP' 'BY' Expression (',' Expression)*
ReturnClause      ::= 'RETURN' ResultClasses '{' (ReturnItem (',' ReturnItem)* ','?)? '}'
ResultClasses     ::= '[' (Name (',' Name)*)? ']'                 (* untyped, or one or more *)
                    | Name                                        (* single class, no brackets *)
                    | (* empty — go straight to '{' *)
OrderBy           ::= 'ORDER' 'BY' OrderItem (',' OrderItem)*
Limit             ::= 'LIMIT' Integer
Top               ::= 'TOP' Integer
Offset            ::= 'OFFSET' Integer
Distinct          ::= 'DISTINCT'

(* Patterns *)
Pattern           ::= 'NOT'? (ClassRef '(' Variable ')' | Variable) ObjectPattern
ObjectPattern     ::= '{' (PropertyPattern (',' PropertyPattern)* ','?)? '}'
PropertyPattern   ::= Name ':' (Variable | Literal | ArrayPattern)
ArrayPattern      ::= '[' (Variable (',' Variable)* (',' '...')?)? ']'  (* D59: Exact | AtLeast *)
                    | '[' '...' Variable '...' ']'   (* D59 Each: one binding per element *)

(* Names *)
Name              ::= Identifier | StringLit          (* a StringLit must parse as a full IRI *)
ClassRef          ::= Identifier | StringLit
InstitutionRef    ::= Identifier | StringLit
QueryClass        ::= Identifier | StringLit

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
RelationalExpr    ::= AdditiveExpr '~' AdditiveExpr HintSet?   (* D43; LHS must be a Variable *)
                    | AdditiveExpr (RelOp AdditiveExpr)*
RelOp             ::= '<' | '<=' | '>' | '>=' | 'IN' | 'NOT' 'IN' | 'LIKE' | 'NOT' 'LIKE'
HintSet           ::= '{' (Hint (',' Hint)*)? '}'
Hint              ::= 'via' ':' ('text' | 'vector' | 'hybrid')
                    | 'model' ':' StringLit
                    | 'k' ':' Integer
                    | 'limit' ':' Integer
AdditiveExpr      ::= MultiplicativeExpr (('+' | '-' | '||') MultiplicativeExpr)*
MultiplicativeExpr::= PowerExpr (('*' | '/' | '%') PowerExpr)*
PowerExpr         ::= UnaryExpr ('**' UnaryExpr)*             (* left-associative *)
UnaryExpr         ::= ('NOT' | '+' | '-') UnaryExpr
                    | 'NOT' 'EXISTS' '(' Variable ')'
                    | VerdictTerm
VerdictTerm       ::= PrimaryExpr ('HOLDS' | 'FAILS' | 'UNDECIDABLE')?  (* non-associative *)
PrimaryExpr       ::= '(' Expression ')'
                    | '[' ArgList? ']'                        (* array literal; may be empty *)
                    | ScalarFn '(' ArgList ')'
                    | AggregateFn '(' Expression ')'
                    | Variable ('.' Identifier)*              (* variable, or a dot-path *)
                    | (StringLit | QualifiedName | Identifier) '(' ArgList ')'
                    | QualifiedName | Identifier              (* bare name: its own text *)
                    | Literal
ArgList           ::= Expression (',' Expression)*   (* never empty — no zero-arg call parses *)
QualifiedName     ::= Identifier ':' Identifier
Literal           ::= StringLit | Integer | Float | 'true' | 'false'
ScalarFn          ::= 'DATE' | 'TIMESTAMP' | 'REGEX' | 'LENGTH' | 'CONTAINS' | 'CONCAT'
AggregateFn       ::= 'COUNT' | 'SUM' | 'AVG' | 'MIN' | 'MAX'
```

**Notes on the form above**

- **The trailing clauses are positional.** `parse_query` tests for `GROUP BY`, `RETURN`, `ORDER BY`, `LIMIT`, `TOP`, `OFFSET` and `DISTINCT` in exactly that order, once each, and then requires end of input. `TOP 20 LIMIT 5` does not parse; `LIMIT 5 TOP 20` does (and is then rejected at typecheck, which forbids `TOP` with `LIMIT`). Any keyword out of order surfaces as `unexpected token after query body`.
- **`USING` and `USING NAMESPACE` do different jobs.** `USING "<class-iri>"` asserts that an IRI resolves to a `Class`; it is checked at typecheck and does nothing else. Bare short names (`MATCH Dog(?d)`) resolve against the **core namespace plus every prefix imported by `USING NAMESPACE "<prefix>"`** — see [`kernel/src/query/resolve.rs`](../../../kernel/src/query/resolve.rs) and [chapter 4 §4.2](04-program-structure.md#42-using--class-imports-and-using-namespace--short-name-scope). A short name matching two imported-namespace resources is an ambiguity error, not a first-wins pick.
- `MatchPart` allows any interleaving of the three `USING` forms — they're not ordered into phases. `DefineBody` is the restricted form used in `DEFINE` rules: it permits `USING` and `USING NAMESPACE` but neither `USING INSTITUTION` nor `FIBER` (parser enforces this; see [chapter 10](10-stratification.md)).
- `FIBER` uses `':'` between the institution reference and the query class, not `'.'`. The optional `INTO "<iri>"` suffix pins the response resource at a named chain IRI (D14 §9.3) — see [chapter 8 §8.6](08-fiber-clauses.md).
- **The similarity operator does not chain.** `~` is handled before the relational loop is entered, so `?a ~ "x" ~ "y"` is a parse error, and its left-hand side must be a bare variable — any other expression fails with `similarity LHS must be a property-bound variable`. Its hint set is closed: exactly the keys `via`, `model`, `k`, `limit`, with `via` restricted to `text` / `vector` / `hybrid`, all rejected at parse time rather than at typecheck.
- **A verdict predicate is postfix and consumed once**, so `?v HOLDS FAILS` is rejected.
- **`ArgList` is never empty.** Every argument list runs through `parse_expression_list`, which parses one expression before looking for a comma, so `LENGTH()` and `ns:f()` are parse errors. An empty *array literal* `[]` is fine — that is a different production.
- A function name may be a quoted IRI: `"urn:eigenius:dock:within_tolerance"(?d, 2.0)` parses as a call, matching the `qualified_name ::= IDENTIFIER ':' IDENTIFIER | STRING` rule of D2 §3.8. A `StringLit` *not* followed by `(` is an ordinary string literal.
- `WHERE` accepts a comma-separated expression list, all implicitly ANDed.
- `ReturnClause`'s class spec has three forms: bracketed name list (possibly empty), a single bare name, or omitted entirely (braces directly after `RETURN`).
- `IN`, `NOT IN`, `LIKE`, and `NOT LIKE` accept any `AdditiveExpr` on the right — typically an array literal for `IN` and a string for `LIKE`, but a variable bound to a list/string is also valid.
- Equality and relational chains are written as `*` to match the parser, but consecutive non-associative comparisons are unusual; pre-formed chains like `?a = ?b = ?c` are valid by grammar, evaluated left-associatively.
- `**` binds *looser* than the unary operators and folds **left**: `parse_power_expr` calls `parse_unary_expr` on each side and loops rather than recursing, so `-a ** b` is `(-a) ** b` and `2 ** 3 ** 2` is `64`, not `512`. Parenthesise stacked exponents.
- `Expression::Object` exists in the AST and has no production: object literals in expression position do not parse, and the evaluator's arm for the variant reports `object literals in expressions not yet implemented`.
- A bare `Identifier` in expression position evaluates to the identifier text as a string literal (used to pass shortnames as values, e.g., in `RETURN`).
- **A `-` immediately before a digit lexes as part of the number**, so `?a -1` is two adjacent tokens (`?a`, `-1`) with no operator between them and fails at end of input. Write `?a - 1`.

## 13.2. Keyword reference

The lexer's keyword table has 42 arms: 40 keywords and the two boolean literals. All keywords are UPPERCASE and case-sensitive except `true` / `false`.

### Structural keywords

`USING`, `NAMESPACE`, `INSTITUTION`, `AS`, `DEFINE`, `FROM`, `MATCH`, `FIBER`, `INTO`, `WHERE`, `RETURN`, `GROUP`, `BY`, `ORDER`, `ASC`, `DESC`, `DISTINCT`, `LIMIT`, `OFFSET`, `TOP`

### Operator keywords

`AND`, `OR`, `NOT`, `IN`, `LIKE`, `EXISTS`, `HOLDS`, `FAILS`, `UNDECIDABLE`

### Built-in function keywords

`DATE`, `TIMESTAMP`, `REGEX`, `LENGTH`, `CONTAINS`, `CONCAT`, `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`

### Literal keywords

`true`, `false` (lowercase)

### Not keywords

The similarity hint keys `via`, `model`, `k`, `limit`, and the hint values `text`, `vector`, `hybrid`, are ordinary identifiers matched by position inside a hint set. `via` and `k` are usable as short names elsewhere.

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

None of the eleven accepts a zero-argument call: `COUNT()` and `LENGTH()` are parse errors.

## 13.4. Operator precedence table

From tightest (evaluated first) to loosest. This table is derived from the descent order in `parse_expression` and agrees with the EBNF in §13.1.

| Level | Operators | Associativity |
|---|---|---|
| 1 | Primary: literals, variables, `(...)`, function calls, aggregates, `[...]`, dot-paths | — |
| 2 | Postfix `HOLDS`, `FAILS`, `UNDECIDABLE` | non-associative |
| 3 | Unary `NOT`, `+`, `-`, `NOT EXISTS` | right |
| 4 | `**` (power) | **left** |
| 5 | `*` `/` `%` | left |
| 6 | `+` `-` `\|\|` | left |
| 7 | `<` `<=` `>` `>=` `IN` `NOT IN` `LIKE` `NOT LIKE`, and `~` | left; `~` does not chain |
| 8 | `=` `<>` | left |
| 9 | `AND` | left |
| 10 | `OR` | left |

Two departures from the conventional table are worth memorising, because both change results silently:

- Unary binds **tighter** than `**`: `- ?a ** 2` parses as `(- ?a) ** 2`, not `-(?a ** 2)`. `parse_power_expr` calls `parse_unary_expr` on each side, so the minus is consumed before the power loop is entered.
- `**` folds **left**, where mathematics folds right: `2 ** 3 ** 2` is `(2 ** 3) ** 2` = `64`, not `2 ** (3 ** 2)` = `512`.

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
- [kernel/src/query/evaluate/](../../../kernel/src/query/evaluate/) — evaluator, split by phase: `mod.rs` (strata + fixpoint),
  `pattern.rs` (pattern match, candidate collection), `expression.rs` (expressions, aggregates),
  `fiber.rs` (FIBER dispatch, overlay), `similarity.rs` (`~` pre-pass), `return_shape.rs` (RETURN, DISTINCT, ORDER BY)
- [kernel/src/query/resolve.rs](../../../kernel/src/query/resolve.rs) — namespace-scoped short-name resolution (`USING NAMESPACE`)
- [kernel/src/query/functions.rs](../../../kernel/src/query/functions.rs) — built-in function dispatch and helpers
- [kernel/src/query/document.rs](../../../kernel/src/query/document.rs) — result-document shaping
- [kernel/src/query/error.rs](../../../kernel/src/query/error.rs) — `QueryError`

**Institution module** (D14):
- [kernel/src/institution/runtime.rs](../../../kernel/src/institution/runtime.rs) — `Institution` trait, `InstitutionRuntime`
- [kernel/src/institution/registry.rs](../../../kernel/src/institution/registry.rs) — `InstitutionIndex` (derived from chain scan)
- [kernel/src/institution/dispatch.rs](../../../kernel/src/institution/dispatch.rs) — `AutoOnLoad` dispatch
- [kernel/src/institution/error.rs](../../../kernel/src/institution/error.rs) — `InstitutionError`
- [kernel/src/capability/registration.rs](../../../kernel/src/capability/registration.rs) — chain-scan auto-registration of WASM institutions / components
- `kernel/src/capability/wasm_institution_d14.rs` *(deleted `2026-07-08` with the WASM path)* — host bridge to the `eigenius-institution-d14` WIT world

**Core / institution ontology**:
- [ontologies/core/core-ontology.json](../../../ontologies/core/core-ontology.json) — shipped definitions of `Class`, `Property`, `Verdict`, etc.
- [ontologies/institution/institution-ontology.json](../../../ontologies/institution/institution-ontology.json) — `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`

---

Return to **[README](README.md)**.
