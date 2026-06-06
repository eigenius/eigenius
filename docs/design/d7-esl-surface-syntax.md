# D7: ESL — Eigenius Surface Language

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 4.5)
**Required before:** Phase 4.5 implementation
**Resolves:** Concrete syntax for programs, ontologies, and resource definitions

---

## 1. Overview

ESL is a human-friendly surface syntax that compiles to Eigon-JSON. It covers three domains:

1. **Programs** — typed expressions (let, apply, lambda, case, construct, etc.)
2. **Ontologies** — class and property definitions with constraints
3. **Resources** — instance data with type annotations

ESL is pure syntactic sugar. Every ESL construct has a 1:1 mapping to Eigon-JSON. The compiler produces Eigon-JSON that the kernel processes unchanged.

---

## 2. Design Principles

- **Eigon-JSON is the truth.** ESL is convenience, not a separate language.
- **Familiar syntax.** Draws from ML/Haskell for programs, protobuf/GraphQL for ontologies.
- **Whitespace-insensitive.** Braces and semicolons delimit blocks, not indentation.
- **IRIs are first-class.** Full URN IRIs can appear anywhere, with namespace aliases for brevity.
- **One file = one compilation unit.** An `.esl` file produces one Eigon-JSON array.

---

## 3. Namespace Aliases

Every ESL file can declare namespace aliases:

```esl
namespace core = "urn:eigenius:core";
namespace prog = "urn:eigenius:program";
namespace ex = "urn:eigenius:example";
```

An identifier `ex:Dog` expands to `"urn:eigenius:example:Dog"`. Bare identifiers (no colon) expand using context — class names resolve against imported namespaces.

---

## 4. Program Syntax

### 4.1 Program Declaration

```esl
program extract_and_summarize : ex:Document -> ex:Summary {
  let entities : ex:Entities = CompleteJson(input.ex:text, ex:extract_prompt);
  let summary : core:string = CompleteText(input.ex:text, ex:summarize_prompt);
  Construct ex:Summary {
    entities = entities,
    summary = summary,
    source = input
  }
}
```

Desugars to:
```json
{
  "@id": "urn:eigenius:example:extract_and_summarize",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
  "urn:eigenius:program:input_type": "urn:eigenius:example:Document",
  "urn:eigenius:program:output_type": "urn:eigenius:example:Summary",
  "urn:eigenius:program:body": { ... }
}
```

### 4.2 Expression Forms

| ESL | Expression Class | Example |
|-----|-----------------|---------|
| `let x : T = e1; e2` | Let | `let name : string = Identity(input); name` |
| `f(arg)` | Apply | `CompleteText(input, prompt)` |
| `x` | Var | `input` |
| `\x -> e` or `λx -> e` | Lambda | `\doc -> doc.text` |
| `case e { C1 -> e1; C2 -> e2 }` | Case | `case result { Ok -> v; Err -> fallback }` |
| `(e1, e2)` | Pair | `(name, age)` |
| `Construct C { f1 = e1, f2 = e2 }` | Construct | `Construct Dog { name = "Rex" }` |
| `e.prop` | Project | `input.ex:name` |
| `map(f, collection)` | Map | `map(\x -> process(x), items)` |
| `reduce(f, init, collection)` | Reduce | `reduce(\acc x -> combine(acc, x), empty, items)` |
| `42`, `"hello"`, `true` | Literal | — |

### 4.3 Operator Precedence (tightest first)

1. `.` (projection)
2. Function application `f(arg)`
3. `\x -> e` (lambda, extends as far right as possible)
4. `let ... ; ...` (let binding, extends as far right as possible)
5. `case ... { ... }` (case expression)

### 4.4 Component Calls

Component calls look like function application. The function name resolves to a component IRI:

```esl
CompleteText(input, prompt)
```

Desugars to an Apply with the component IRI as function. The function name is resolved against registered components:
- `CompleteText` → `urn:eigenius:program:components:CompleteText`
- A fully qualified `prog:components:CompleteText` also works.

---

## 5. Ontology Syntax

### 5.1 Class Definitions

```esl
class ex:Document {
  description = "A text document for analysis";
  requires ex:text;
  recommends ex:author, ex:date;
}
```

### 5.2 Property Definitions

```esl
property ex:text : core:string {
  description = "The text content of a document";
}

property ex:count : core:integer {
  description = "Number of items";
  min_value = 0;
  max_value = 1000;
}

property ex:status : core:resource {
  description = "Current status";
  allows_only = [ex:StatusActive, ex:StatusInactive];
  domain = [ex:Document];
}
```

### 5.3 Subclassing

```esl
class ex:Dog : ex:Animal {
  description = "A dog";
  requires ex:breed;
}
```

The `: ex:Animal` desugars to `subclass_of: ["urn:eigenius:example:Animal"]`.

---

## 6. Resource Syntax

```esl
resource ex:rex : ex:Dog {
  ex:name = "Rex";
  ex:breed = "German Shepherd";
}
```

Desugars to:
```json
{
  "@id": "urn:eigenius:example:rex",
  "urn:eigenius:core:is_a": ["urn:eigenius:example:Dog"],
  "urn:eigenius:example:name": "Rex",
  "urn:eigenius:example:breed": "German Shepherd"
}
```

---

## 7. Grammar (EBNF)

```ebnf
file          = { namespace_decl } { top_level } ;
namespace_decl = "namespace" IDENT "=" STRING ";" ;

top_level     = program_decl | class_decl | property_decl | resource_decl ;

(* Programs *)
program_decl  = "program" qualified_name ":" type "->" type "{" expr "}" ;
expr          = let_expr | lambda_expr | case_expr | apply_expr ;
let_expr      = "let" IDENT ":" type "=" expr ";" expr ;
lambda_expr   = ("\\" | "λ") IDENT "->" expr ;
case_expr     = "case" expr "{" { branch } "}" ;
branch        = IDENT "->" expr ";" ;
apply_expr    = atom_expr | atom_expr "(" expr { "," expr } ")" ;
atom_expr     = qualified_name | literal | "(" expr ")" | construct_expr | project_expr ;
construct_expr = "Construct" qualified_name "{" { field } "}" ;
field         = IDENT "=" expr "," ;
project_expr  = atom_expr "." qualified_name ;

(* Ontology *)
class_decl    = "class" qualified_name [ ":" qualified_name ] "{" { class_body } "}" ;
class_body    = "description" "=" STRING ";"
              | "requires" qualified_name { "," qualified_name } ";"
              | "recommends" qualified_name { "," qualified_name } ";" ;

property_decl = "property" qualified_name ":" qualified_name "{" { prop_body } "}" ;
prop_body     = "description" "=" STRING ";"
              | "min_value" "=" NUMBER ";"
              | "max_value" "=" NUMBER ";"
              | "min_length" "=" NUMBER ";"
              | "max_length" "=" NUMBER ";"
              | "format" "=" qualified_name ";"
              | "pattern" "=" STRING ";"
              | "allows_only" "=" "[" qualified_name { "," qualified_name } "]" ";"
              | "domain" "=" "[" qualified_name { "," qualified_name } "]" ";" ;

(* Resources *)
resource_decl = "resource" qualified_name ":" qualified_name "{" { resource_body } "}" ;
resource_body = qualified_name "=" value ";" ;

(* Common *)
qualified_name = IDENT [ ":" IDENT ] ;
type          = qualified_name ;
value         = STRING | NUMBER | BOOL | qualified_name | "[" value { "," value } "]" ;
literal       = STRING | NUMBER | BOOL ;
```

---

## 8. Error Reporting

ESL source locations (line, column) are tracked through compilation. When the kernel reports a validation error on an IRI, the CLI maps it back to the ESL source location if available.

---

## 9. Implementation Plan

1. **Lexer** (`kernel/src/esl/lexer.rs`) — tokenize ESL source, reuse Position type from EigenQL
2. **Parser** (`kernel/src/esl/parser.rs`) — recursive descent, produces `EslAst` (not Eigon-JSON directly)
3. **Compiler** (`kernel/src/esl/compile.rs`) — walk AST, emit Eigon-JSON resources
4. **CLI** — `eigenius compile file.esl` outputs Eigon-JSON; `eigenius run file.esl input.json` compiles inline

---

## 10. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Indentation-sensitive? | No — braces and semicolons | Avoids parser complexity; easier to embed in other formats |
| Lambda syntax | `\x -> e` and `λx -> e` both accepted | `\` is ASCII-friendly; `λ` is readable |
| Semicolons | Required after let bindings and definitions | Unambiguous parsing without lookahead |
| Type annotations | Required on let and program declarations | Explicit types match EigenTT; aids error messages |
| Component resolution | Short names resolved against program ontology | `CompleteText` → `urn:eigenius:program:components:CompleteText` |
| File extension | `.esl` | Distinct from `.json`, `.eigenql` |
