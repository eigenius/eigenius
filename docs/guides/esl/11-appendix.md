# 11. Appendix

## 11.1. Grammar reference

[D7 §7](../../design/d7-esl-surface-syntax.md#7-grammar-ebnf) is the design document for the grammar; the productions below are read off [`kernel/src/esl/parser.rs`](../../../kernel/src/esl/parser.rs) and are current as of 2026-08-20, including the post-D7 additions (institution-capability syntax from Phase 11e.1, `axiom` from eigenius#72, `def` from D66, `macro` from D52 §12, `merge_comorphism` from D37 §3.3, the index telescope from eigenius#72 Layer 2).

### 11.1.1. Top-level

```ebnf
File         ::= (NamespaceDecl | Declaration)*    (* freely interleaved *)
NamespaceDecl::= 'namespace' Identifier '=' StringLit ';'

Declaration  ::= ClassDecl | PropertyDecl | ResourceDecl
              |  DataDecl  | ProgramDecl
              |  DefDecl                                   (* D66 — §11.1.3 *)
              |  AxiomDecl                                 (* eigenius#72 *)
              |  MacroDecl                                 (* D52 §12 *)
              |  MergeComorphismDecl                       (* D37 §3.3 *)
              |  TextIndexDecl | VectorIndexDecl           (* parse only — see below *)

AxiomDecl    ::= 'axiom' QualifiedName ':' TypeExpr AxiomNote* ';'?
AxiomNote    ::= ('desc' | 'note') ':' StringLit    (* core:description / axiom_justification *)
MacroDecl    ::= 'macro' QualifiedName '(' (MacroParam (',' MacroParam)* ','?)? ')'
                 ':' TypeExpr '=>' Value ';'?
MacroParam   ::= Identifier ':' TypeExpr
MergeComorphismDecl
             ::= 'merge_comorphism' QualifiedName 'for' QualifiedName
                 '{' (MergeInline | MergeReference) '}'
MergeInline  ::= '(' Identifier (',' Identifier)* ')' '=>' Expr
MergeReference::= 'transformation' '=' QualifiedName ';'?
TextIndexDecl  ::= 'text_index'   QualifiedName '{' ResourceField* '}'
VectorIndexDecl::= 'vector_index' QualifiedName '{' ResourceField* '}'
```

The file loop dispatches on each token in turn, so `namespace` aliases and declarations may appear in any order — there is no namespace phase followed by a declaration phase.

**`text_index` and `vector_index` lex and parse but do not compile.** `compile_declaration` returns `text_index lowering not yet implemented (D43 M2)` / `vector_index lowering not yet implemented (D43 M2)`. They are the only two forms the parser accepts and the compiler cannot emit; write the `core:TextIndex` / `core:VectorIndex` resource declaration directly instead.

`transformation` in a `merge_comorphism` reference body is a *contextual* identifier, not a reserved keyword: it is matched by name at that one position and remains a free identifier everywhere else.

### 11.1.2. Class, property, resource

```ebnf
ClassDecl     ::= 'class' QualifiedName (':' QualifiedName (',' QualifiedName)*)?
                  '{' ClassItem* '}'
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
              |  'element_type' '=' QualifiedName ';'
              |  'allows_only' QualifiedName (',' QualifiedName)* ';'   (* no '=' *)
              |  'domain'      QualifiedName (',' QualifiedName)* ';'   (* no '=' *)
              |  'class_types' QualifiedName (',' QualifiedName)* ';'   (* no '=' *)

ResourceDecl  ::= 'resource' QualifiedName ':' QualifiedName (',' QualifiedName)*
                  '{' ResourceField* '}'
ResourceField ::= QualifiedName '=' Value ';'

Value         ::= StringLit | Integer | Float | Boolean
              |  '-' (Integer | Float)                      (* literals only *)
              |  QualifiedName                              (* IRI ref *)
              |  '[' Value (',' Value)* ','? ']'            (* array *)
              |  '{' ResourceField* '}'                     (* embedded resource *)
              |  Identifier '(' (Value (',' Value)* ','?)? ')'   (* inductive ctor *)
              |  QualifiedName '(' (Value (',' Value)* ','?)? ')' (* macro call, D52 *)
              |  'formula' '(' FormulaExpr ')'              (* Pratt-parsed arithmetic *)
              |  'type_expr' '(' TypeExpr ')'               (* D47-encoded type *)
```

Three asymmetries in `PropertyItem` are worth memorising: `allows_only`, `domain` and `class_types` take **no `=`** — they are bare comma-separated name lists, like `requires` and `recommends` in a class body — and `allows_only` takes qualified *names*, not values.

`class` and `resource` both take a comma-separated list after the colon (eigenius#29): a class may declare several superclasses, a resource several classes. **A superclass can only be authored in the header** — `parse_class_item` accepts `description`, `requires` and `recommends` only, and a body-level `subclass_of` item is rejected as an unknown class item.

A nullary constructor in value position requires empty parens (`Foo()`); a bare `Foo` is a resource reference. Unary `-` applies to numeric literals only; arithmetic on references lives inside `formula(...)`.

### 11.1.3. Data and definitions

```ebnf
DataDecl     ::= 'data' QualifiedName ParamList?
                 (':' IndexTelescope)?               (* eigenius#72 Layer 2 *)
                 (',' QualifiedName)*                (* extra is_a classes, D52 §12 *)
                 '{' (Ctor (',' Ctor)* ','?)? '}'    (* zero ctors is legal *)
ParamList    ::= '(' DataParam (',' DataParam)* ')'
DataParam    ::= Identifier ':' (QualifiedName | Sort)   (* "A : core:Set", "P : Prop" *)
IndexTelescope::= (TypeExpr '->')* Sort              (* "Nat -> Set" *)
Sort         ::= 'Prop' | 'Set' | 'Type' Integer

Ctor         ::= Identifier                          (* nullary, e.g. "zero" *)
              |  Identifier '(' CtorArg (',' CtorArg)* ')'
              |  Identifier ':' TypeExpr             (* typed form *)

CtorArg      ::= CtorArgType                         (* positional / anonymous *)

CtorArgType  ::= QualifiedName                       (* "Nat" *)
              |  QualifiedName '(' CtorArgType (',' CtorArgType)* ')'  (* "List(A)" *)


TypeExpr     ::= QualifiedName                                           (* "Nat" *)
              |  QualifiedName '(' TypeExpr (',' TypeExpr)* ')'          (* "List(A)" *)
              |  TypeExpr '->' TypeExpr                                  (* "A -> B" *)
              |  ('pi' | 'forall') TypedParams '=>' TypeExpr             (* Π — "forall (x : T) => B" *)
              |  'exists' TypedParams '=>' TypeExpr                      (* Σ — "exists (x : T) => B" *)
              |  'fun' TypedParams '=>' TypeExpr                         (* λ in type position — match motive *)
              |  'Prop' | 'Set' | 'Type' Integer                         (* sorts, D46 §2 *)
              |  '(' ')'                                                 (* unit VALUE, Exp::Unit *)
              |  '(' TypeExpr ':' TypeExpr ')'                           (* annotation — mode switch *)
              |  'alias' AliasBinding (',' AliasBinding)* 'in' TypeExpr  (* compile-time substitution *)
              |  StringLit | Integer | Float | 'true' | 'false'          (* Exp::LitString / LitInt / LitFloat / LitBool *)

TypedParams  ::= TypedParam (',' TypedParam)*
              |  '(' TypedParam (',' TypedParam)* ')'   (* outer parens optional *)
TypedParam   ::= Identifier ':' TypeExpr
AliasBinding ::= Identifier '=' TypeExpr                (* later bindings see earlier ones *)

DefDecl      ::= 'def' QualifiedName
                 ('(' TypedParam (',' TypedParam)* ')')?
                 ':' TypeExpr '=' TypeExpr
                 ('desc' ':' StringLit)? ';'?           (* D66 transparent definition, §4.4c *)
```

`pi` / `forall` / `exists` / `fun` and the sort and unit forms are accepted wherever `parse_type_expr` runs — `axiom` statements, `def` result types and bodies, `data` constructor types, and [`type_expr(...)`](05-expressions.md#5-14a-type_expr-eigentt-type-expressions) blocks. `exists` and `()` reach the kernel only through the `type_expr` lowering path; the resource-shaped type language ([§6](06-resources-types-and-the-layer.md)) has no encoding for either and the compiler rejects them there. `eigentt:fst(p)` / `eigentt:snd(p)` parse as ordinary applications and are intercepted at lowering into `Exp::Fst` / `Exp::Snd`.

### 11.1.4. Program and expressions

```ebnf
ProgramDecl  ::= 'program' QualifiedName ':' QualifiedName '->' QualifiedName
                 '{' ProgramAttribute* Expr '}'
ProgramAttribute ::= 'description' '=' StringLit ';'

Expr         ::= LetExpr | LambdaExpr | TypedLambdaExpr | CaseExpr | MatchExpr
              |  ConstructExpr | ApplyExpr | ProjectExpr
              |  MapExpr | ReduceExpr
              |  PairExpr | LiteralExpr | VarExpr

LetExpr      ::= 'let' Identifier ':' QualifiedName '=' Expr ';' Expr
LambdaExpr   ::= ('\' | 'λ') Identifier '->' Expr          (* untyped *)
TypedLambdaExpr
             ::= 'lambda' TypedParam (',' TypedParam)* '=>' Expr   (* D37 §3.1 *)
CaseExpr     ::= 'case' Expr '{' (Identifier '->' Expr ';')* '}'
MatchExpr    ::= 'match' Expr ('returning' TypeExpr)? '{' MatchArm (';' MatchArm)* ';'? '}'
MatchArm     ::= Identifier ('(' Identifier (',' Identifier)* ')')? '->' Expr
MapExpr      ::= 'map' '(' Expr ',' Expr ')'               (* recognised by name + arity 2 *)
ReduceExpr   ::= 'reduce' '(' Expr ',' Expr ',' Expr ')'   (* by name + arity 3 *)

ConstructExpr::= 'Construct' QualifiedName '{' (QualifiedName '=' Expr (',' QualifiedName '=' Expr)*)? '}'

ApplyExpr    ::= QualifiedName '(' (Expr (',' Expr)*)? ')' ('{' ResourceField* '}')?
ProjectExpr  ::= Expr '.' QualifiedName

PairExpr     ::= '(' Expr ',' Expr ')'
LiteralExpr  ::= StringLit | Integer | Float | Boolean
VarExpr      ::= Identifier

QualifiedName::= Identifier ':' Identifier
              |  Identifier                          (* bare; resolved by context *)
```

**D14 addition**: `ApplyExpr` with a `QualifiedName` whose IRI classifies through the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) as a `Decidable` `QueryClass` dispatches as `Exp::NativeDecide` (returning a `Verdict`) — see [§9.3](09-institutions.md). An IRI classifying as a `Comorphism` is also callable from expression position: it lowers to `program:ComorphismInvokeApply`, decoded to `Exp::InstitutionInvoke` (D14 §9.3) — see [§9.5](09-institutions.md#95-invoking-comorphisms-from-esl-programs). Exactly one source argument and no configuration block.

**Application does not curry.** A term takes a projection chain and then at most one argument list: `f(a)(b)` is not a production, and the callee must reduce to a plain or projected name. `map` and `reduce` are recognised at the application site by name *together with* argument count (two and three); any other arity falls through to ordinary application.

**`returning` takes a full `TypeExpr`**, not just a qualified name — including a `fun (i : T) => body` motive for an indexed inductive (eigenius#72 Layer 3). When omitted, the kernel synthesises the motive from the checking-mode expected type.

## 11.2. Keyword reference

### Declaration keywords

`namespace`, `class`, `property`, `resource`, `data`, `program`, `axiom`, `def` (+ `desc`), `macro`, `merge_comorphism` (+ `for`), `text_index`, `vector_index`

### Type and binder keywords

`pi`, `forall`, `exists`, `fun`, `alias` (+ `in`), `Prop`, `Set`, `Type`

### Class/property body keywords

`description`, `requires`, `recommends`, `min_value`, `max_value`, `min_length`, `max_length`, `pattern`, `format`, `allows_only`, `domain`, `class_types`, `element_type`

### Expression keywords

`let`, `case`, `match`, `returning`, `Construct`, `map`, `reduce`, `lambda`

### Literal keywords

`true`, `false`

The lexer's keyword table has 34 arms: the 32 keywords above plus `true` / `false`. **Sixteen of them double as ordinary identifiers** — `expect_ident` accepts `namespace`, `class`, `property`, `resource`, `program`, `let`, `case`, `Construct`, `map`, `reduce`, `Prop`, `Set`, `Type`, `axiom`, `def` and `forall` as names, which is what lets the core ontology hold a resource at `urn:eigenius:core:Set` and a file write `core:property` as a name. The other sixteen are reserved absolutely.

`desc`, `note` (in an `axiom`) and `transformation` (in a `merge_comorphism` reference body) are **contextual identifiers**, matched by name at one position each and free everywhere else. They are not in the keyword table.

## 11.3. Operator/punctuation reference

| Token | Use |
|---|---|
| `=` | Assignment in `let`, namespace, fields |
| `->` | Function-type arrow |
| `=>` | Closes a binder list (`pi` / `forall` / `exists` / `fun`) |
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
| `+` `-` `*` `/` `^` | Arithmetic, **only inside `formula(...)`** — `^` right-associative, the rest left |
| `-` | Unary minus on a numeric literal in value position; unary minus inside `formula(...)` |

## 11.4. Compile API

Four entry points, from [`kernel/src/esl/mod.rs`](../../../kernel/src/esl/mod.rs):

```rust
pub fn compile(source: &str) -> Result<Vec<Resource>, Vec<EslError>>;

pub fn compile_with_institutions(
    source: &str,
    index: Arc<InstitutionIndex>,
) -> Result<Vec<Resource>, Vec<EslError>>;

pub fn compile_against_layer(
    source: &str,
    layer: &Layer,
) -> Result<Vec<Resource>, Vec<EslError>>;

pub fn compile_full(
    source: &str,
    institutions: Arc<InstitutionIndex>,
    layer: &Layer,
) -> Result<Vec<Resource>, Vec<EslError>>;
```

| Entry point | Institution index | Chain ctors + macros |
|---|---|---|
| `compile` | — | — |
| `compile_with_institutions` | yes | — |
| `compile_against_layer` | — | yes |
| `compile_full` | yes | yes |

- `compile` — base path. Use for ontologies, resources, and programs that neither invoke institution-dispatched function calls nor reference declarations from a parent layer.
- `compile_with_institutions` — required for programs that use `cap:predicate(...)` calls referencing a `Decidable` `QueryClass`, or a qualified call naming a `Comorphism`. The `InstitutionIndex` is derived from the layer chain (`InstitutionIndex::from_layer`); the compiler consults it to classify the IRI. Without the index, the compile falls through to ordinary component dispatch and fails at runtime with `unknown function`.
- `compile_against_layer` — adds `collect_ctors_from_layer` + `collect_macros_from_layer`, so a bare-name constructor or a `macro` declared in a parent layer resolves. This is the path the bootstrap uses.
- `compile_full` — both. **This is what the running server calls** for `eigenius load` and notebook-cell ESL (`server/mod.rs` falls back to `compile_with_institutions` only when no layer is available).

## 11.5. Kernel capability modes

[`EvalCtx`](../../../kernel/src/nbe/eval/mod.rs) has **two** variants. The finer capability tier is a property of the attached [`EffectHooks`](../../../kernel/src/nbe/eval/hooks.rs) implementation, not of the enum:

| Context | Layer | Institutions | Components |
|---|---|---|---|
| `EvalCtx::Pure` | — | — | — |
| `EvalCtx::Effectful` + `InstitutionEngine::for_check` | optional | yes | — |
| `EvalCtx::Effectful` + `InstitutionEngine::for_io` | yes | yes | yes |

Constructed via `EvalCtx::Pure` / `EvalCtx::pure()` and `EvalCtx::effectful(layer, hooks)`. See [chapter 8](08-capability-modes.md) for the per-form behaviour table.

## 11.6. Related documents

- [D7 ESL surface syntax](../../design/d7-esl-surface-syntax.md) — authoritative grammar and design
- [D18 Ontology-as-types resolution](../../design/d18-ontology-as-types-resolution.md) — the bridge specified in [chapter 6](06-resources-types-and-the-layer.md)
- [D19 Inductive types](../../design/d19-inductive-types.md) — type theory underpinning [chapter 4](04-declarations.md) (data) and [chapter 7](07-type-theory-primer.md)
- [D14 Institution Realisation](../../design/d14-institution-realisation.md) — institution mechanism dispatched in [chapter 9](09-institutions.md). Supersedes D10.
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
- [`kernel/src/program/ground.rs`](../../../kernel/src/program/ground.rs) — `resolve_class_type`, `resolve_property_type`, `collect_properties` (the bridge from chapter 6)

**Kernel — type theory:**
- [`kernel/src/nbe/term.rs`](../../../kernel/src/nbe/term.rs) — `Exp` (terms) and `Decl` definitions
- [`kernel/src/nbe/val.rs`](../../../kernel/src/nbe/val.rs) — `Val` (semantic values), neutrals, closures
- [`kernel/src/nbe/eval/mod.rs`](../../../kernel/src/nbe/eval/mod.rs) — evaluator with `EvalCtx` capability modes
- [`kernel/src/nbe/check/mod.rs`](../../../kernel/src/nbe/check/mod.rs) — type-checker (`check_infer`, `check_check`)
- [`kernel/src/nbe/readback.rs`](../../../kernel/src/nbe/readback.rs) — readback from `Val` to normal-form `Exp`
- [`kernel/src/nbe/positivity.rs`](../../../kernel/src/nbe/positivity.rs) — positivity check for inductive types

**Institutions (D14):**
- [`kernel/src/institution/runtime.rs`](../../../kernel/src/institution/runtime.rs) — `Institution` trait, `InstitutionRuntime`
- [`kernel/src/institution/registry.rs`](../../../kernel/src/institution/registry.rs) — `InstitutionIndex` (derived from chain scan)
- [`kernel/src/institution/dispatch.rs`](../../../kernel/src/institution/dispatch.rs) — `AutoOnLoad` dispatch
- [`kernel/src/institution/error.rs`](../../../kernel/src/institution/error.rs) — `InstitutionError`
- [`kernel/src/capability/registration.rs`](../../../kernel/src/capability/registration.rs) — auto-registration from chain scan
- [`kernel/src/capability/wasm_institution_d14.rs`](../../../kernel/src/capability/wasm_institution_d14.rs) — host bridge to the `eigenius-institution-d14` WIT world

**Core / institution ontology:**
- [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) — shipped definitions of `Class`, `Property`, `InductiveType`, `Verdict`, etc.
- [`ontologies/institution/institution-ontology.json`](../../../ontologies/institution/institution-ontology.json) — `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`

---

Return to **[README](README.md)**.
