# 11. Appendix

## 11.1. Grammar reference

The authoritative grammar is [D7 §7](../../design/d7-esl-surface-syntax.md#7-grammar-ebnf). This appendix summarises the surface forms covered in this guide and notes the post-D7 additions (sized bounded binders from Phase 11h, institution-capability syntax from Phase 11e.1).

### 11.1.1. Top-level

```ebnf
File         ::= NamespaceDecl* Declaration*
NamespaceDecl::= 'namespace' Identifier '=' StringLit ';'

Declaration  ::= ClassDecl | PropertyDecl | ResourceDecl
              |  DataDecl  | CodataDecl   | ProgramDecl
```

### 11.1.2. Class, property, resource

```ebnf
ClassDecl     ::= 'class' QualifiedName (':' QualifiedName)? '{' ClassItem* '}'
ClassItem     ::= 'description' '=' StringLit ';'
              |  'requires'    QualifiedName (',' QualifiedName)* ';'
              |  'recommends'  QualifiedName (',' QualifiedName)* ';'

PropertyDecl  ::= 'property' QualifiedName ':' QualifiedName '{' PropertyItem* '}'
PropertyItem  ::= 'description' '=' StringLit ';'
              |  'min_value'   '=' Number     ';'
              |  'max_value'   '=' Number     ';'
              |  'min_length'  '=' Integer    ';'
              |  'max_length'  '=' Integer    ';'
              |  'pattern'     '=' StringLit  ';'
              |  'format'      '=' QualifiedName ';'
              |  'allows_only' '=' Value (',' Value)* ';'
              |  'domain'      '=' QualifiedName (',' QualifiedName)* ';'
              |  'class_types' '=' QualifiedName (',' QualifiedName)* ';'
              |  'element_type' '=' QualifiedName ';'

ResourceDecl  ::= 'resource' QualifiedName ':' QualifiedName '{' ResourceField* '}'
ResourceField ::= QualifiedName '=' Value ';'

Value         ::= StringLit | Integer | Float | Boolean
              |  QualifiedName                              (* IRI ref *)
              |  '[' Value (',' Value)* ']'                 (* array *)
              |  '{' ResourceField* '}'                     (* embedded resource *)
```

### 11.1.3. Data and codata

```ebnf
DataDecl     ::= 'data' QualifiedName ParamList? '{' Ctor (',' Ctor)* ','? '}'
ParamList    ::= '(' DataParam (',' DataParam)* ')'
DataParam    ::= Identifier ':' QualifiedName       (* e.g. "A : core:Set" *)

Ctor         ::= Identifier                          (* nullary, e.g. "zero" *)
              |  Identifier '(' CtorArg (',' CtorArg)* ')'

CtorArg      ::= CtorArgType                         (* positional / anonymous *)
              |  '{' BoundedBinder '}'               (* sized binder *)

BoundedBinder::= Identifier (':' QualifiedName)? ('<' QualifiedName)?
              (* {j} | {j : Size} | {j < i} | {j : Size < i} *)

CtorArgType  ::= QualifiedName                       (* "Nat" *)
              |  QualifiedName '(' CtorArgType (',' CtorArgType)* ')'  (* "List(A)" *)


CodataDecl   ::= 'codata' QualifiedName ParamList? '{' Observation (';' Observation)* ';'? '}'
Observation  ::= Identifier ':' TypeExpr

TypeExpr     ::= QualifiedName                                           (* "Nat" *)
              |  QualifiedName '(' TypeExpr (',' TypeExpr)* ')'          (* "Stream(A, j)" *)
              |  TypeExpr '->' TypeExpr                                  (* "A -> B" *)
              |  '{' BoundedBinder '}' '->' TypeExpr                     (* "{j < i} -> body" *)
```

### 11.1.4. Program and expressions

```ebnf
ProgramDecl  ::= 'program' QualifiedName ':' QualifiedName '->' QualifiedName
                 '{' ProgramAttribute* Expr '}'
ProgramAttribute ::= 'description' '=' StringLit ';'

Expr         ::= LetExpr | LambdaExpr | CaseExpr | MatchExpr
              |  ConstructExpr | CoRecordExpr | ApplyExpr | ProjectExpr
              |  PairExpr | LiteralExpr | VarExpr

LetExpr      ::= 'let' Identifier ':' QualifiedName '=' Expr ';' Expr
LambdaExpr   ::= ('\' | 'λ') Identifier '->' Expr
CaseExpr     ::= 'case' Expr '{' (Identifier '->' Expr ';')* '}'
MatchExpr    ::= 'match' Expr ('returning' QualifiedName)? '{' MatchArm (';' MatchArm)* ';'? '}'
MatchArm     ::= Identifier ('(' Identifier (',' Identifier)* ')')? '->' Expr

ConstructExpr::= 'Construct' QualifiedName '{' (QualifiedName '=' Expr (',' QualifiedName '=' Expr)*)? '}'
CoRecordExpr ::= 'corecord' '{' (Identifier '=' Expr ';')* '}'

ApplyExpr    ::= QualifiedName '(' (Expr (',' Expr)*)? ')' ('{' ResourceField* '}')?
ProjectExpr  ::= Expr '.' QualifiedName

PairExpr     ::= '(' Expr ',' Expr ')'
LiteralExpr  ::= StringLit | Integer | Float | Boolean
VarExpr      ::= Identifier

QualifiedName::= Identifier ':' Identifier
              |  Identifier                          (* bare; resolved by context *)
```

**Phase 11e.1 addition**: `ApplyExpr` with a `QualifiedName` whose IRI classifies through the institution registry as a decide predicate or comorphism dispatches as the corresponding kernel form ([§9.2](09-institutions.md), [§5.2.3](05-expressions.md), [§5.2.4](05-expressions.md)).

**Phase 11h addition**: brace-delimited bounded binders in constructor argument and codata observation positions. The three accepted shapes are tabulated in [§4.5](04-declarations.md).

## 11.2. Keyword reference

### Declaration keywords

`namespace`, `class`, `property`, `resource`, `data`, `codata`, `program`

### Class/property body keywords

`description`, `requires`, `recommends`, `min_value`, `max_value`, `min_length`, `max_length`, `pattern`, `format`, `allows_only`, `domain`, `class_types`, `element_type`

### Expression keywords

`let`, `case`, `match`, `returning`, `Construct`, `map`, `reduce`, `corecord`

### Literal keywords

`true`, `false`

## 11.3. Operator/punctuation reference

| Token | Use |
|---|---|
| `=` | Assignment in `let`, namespace, fields |
| `->` | Function-type arrow |
| `\` | Lambda (ASCII) |
| `λ` | Lambda (Unicode) |
| `.` | Property projection |
| `;` | Statement / declaration terminator |
| `:` | Type annotation, qualified-name separator, parent class |
| `,` | List separator |
| `<` | Size bound in bounded binders |
| `(` `)` | Function call args, parameter telescopes |
| `{` `}` | Block delimiters (declaration bodies, expression blocks, bounded binders) |
| `[` `]` | Array literals (inside resource fields) |

## 11.4. Compile API

From [`kernel/src/esl/mod.rs`](../../../kernel/src/esl/mod.rs):

```rust
pub fn compile(source: &str) -> Result<Vec<Resource>, Vec<EslError>>;

pub fn compile_with_institutions(
    source: &str,
    institutions: Arc<InstitutionRegistry>,
) -> Result<Vec<Resource>, Vec<EslError>>;
```

- `compile` — base path. Use for ontologies, resources, and programs that do not invoke institution-dispatched function calls.
- `compile_with_institutions` — required for programs that use `cap:predicate(...)` or `cap:translate(...)` — the registry is consulted at compile time to classify the IRI as a decide predicate or comorphism. Without the registry, the compile would fall through to ordinary component dispatch and fail at runtime with `unknown function`.

## 11.5. Kernel capability modes

| Mode | Layer | Components | Institutions |
|---|---|---|---|
| `Pure` | — | — | — |
| `Read` | yes | — | — |
| `Check` | optional | — | yes |
| `IO` | yes | yes | yes |

Constructed via the [`EvalCtx`](../../../kernel/src/nbe/eval.rs) variants. See [chapter 8](08-capability-modes.md) for the full per-mode behaviour table.

## 11.6. Related documents

- [D7 ESL surface syntax](../../design/d7-esl-surface-syntax.md) — authoritative grammar and design
- [D18 Ontology-as-types resolution](../../design/d18-ontology-as-types-resolution.md) — the bridge specified in [chapter 6](06-resources-types-and-the-layer.md)
- [D19 Inductive and sized types](../../design/d19-inductive-types.md) — type theory underpinning [chapter 4](04-declarations.md) (data/codata) and [chapter 7](07-type-theory-primer.md)
- [D11 Codata, streams, and resumable execution](../../design/d11-codata-streams.md) — coinductive type design
- [D10 Grothendieck institution protocol](../../design/d10-grothendieck-institution-protocol.md) — institution mechanism dispatched in [chapter 9](09-institutions.md)
- [D1 Eigon serialization format](../../design/d1-eigon-serialization-format.md) — the resource model ESL compiles to
- [EigenQL user guide](../eigenql/README.md) — the query-language companion sharing the same institution classification

## 11.7. Source index

All implementation referenced in this guide:

**ESL pipeline:**
- [`kernel/src/esl/mod.rs`](../../../kernel/src/esl/mod.rs) — public entry points (`compile`, `compile_with_institutions`)
- [`kernel/src/esl/lexer.rs`](../../../kernel/src/esl/lexer.rs) — tokenizer
- [`kernel/src/esl/parser.rs`](../../../kernel/src/esl/parser.rs) — parser
- [`kernel/src/esl/ast.rs`](../../../kernel/src/esl/ast.rs) — AST types
- [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs) — compiler from AST to Eigon-JSON resources
- [`kernel/src/esl/error.rs`](../../../kernel/src/esl/error.rs) — `EslError`

**Program parsing (resource → kernel `Exp`):**
- [`kernel/src/program/expr.rs`](../../../kernel/src/program/expr.rs) — `parse_program`, `parse_expression`
- [`kernel/src/program/ground.rs`](../../../kernel/src/program/ground.rs) — `resolve_class_type`, `resolve_property_type`, `collect_properties`, `resolve_codata_type` (the bridge from chapter 6)

**Kernel — type theory:**
- [`kernel/src/nbe/term.rs`](../../../kernel/src/nbe/term.rs) — `Exp` (terms) and `Decl` definitions
- [`kernel/src/nbe/val.rs`](../../../kernel/src/nbe/val.rs) — `Val` (semantic values), neutrals, closures
- [`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs) — evaluator with `EvalCtx` capability modes
- [`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs) — type-checker (`check_infer`, `check_check`)
- [`kernel/src/nbe/readback.rs`](../../../kernel/src/nbe/readback.rs) — readback from `Val` to normal-form `Exp`
- [`kernel/src/nbe/positivity.rs`](../../../kernel/src/nbe/positivity.rs) — positivity check for inductive types

**Institutions:**
- [`kernel/src/institution/mod.rs`](../../../kernel/src/institution/mod.rs) — `FiberReasoner`, `InstitutionRegistry`, `DecResult`, `InstitutionCapability`
- [`kernel/src/institution/error.rs`](../../../kernel/src/institution/error.rs) — `InstitutionError`, `MorphismValidation`

**Core ontology:**
- [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) — shipped definitions of `Class`, `Property`, `InductiveType`, `CodataType`, `Comorphism`, etc.

---

Return to **[README](README.md)**.
