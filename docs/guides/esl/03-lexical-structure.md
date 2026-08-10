# 3. Lexical structure

The ESL lexer is a hand-written tokenizer in [`kernel/src/esl/lexer.rs`](../../../kernel/src/esl/lexer.rs). Whitespace and comments are discarded; everything else becomes a `Token`. ESL is whitespace-insensitive — line breaks are not significant — so source layout is purely cosmetic.

## 3.1. Whitespace and comments

Spaces, tabs, carriage returns, and newlines separate tokens but carry no meaning. Two comment forms:

```esl
// line comment to end of line
/* block comment;
   may span multiple lines */
```

Block comments do not nest.

## 3.2. Keywords

Keywords are case-sensitive lowercase, except for `Construct` (it occupies an expression position alongside identifiers and is capitalised to match the convention for type-construction operations) and the sort names `Prop`, `Set`, `Type`.

**Top-level (declaration) keywords** mark the start of a top-level form:

```
namespace  class  property  resource  program  data  codata  axiom  def  macro
merge_comorphism  text_index  vector_index
```

**Expression keywords** appear inside `program` bodies:

```
let  case  match  returning  Construct  map  reduce  corecord  lambda
```

**Type and binder keywords** appear in type positions — `axiom` statements, `def` result types and bodies, `data` constructor types, and [`type_expr(...)`](05-expressions.md#5-14a-type_expr-eigentt-type-expressions) blocks:

```
pi  forall  exists  fun  alias  in  Prop  Set  Type
```

`forall` is an alias for `pi` with proposition-authoring semantics — same AST. `exists` opens the Σ (dependent-pair) binder, its dual ([§5.14a](05-expressions.md#5-14a-type_expr-eigentt-type-expressions)).

**Boolean literals** lex as `BoolLit` tokens:

```
true  false
```

There is no `null`/`undefined` literal — Eigon-JSON uses property absence to express "no value".

## 3.3. Identifiers and qualified names

```
IDENT ::= [a-zA-Z_] [a-zA-Z0-9_]*
```

A bare identifier (`Dog`, `name`, `short_name`) is one `Ident` token. There's no distinction at the lexer level between class names, property names, variable names, and component names — the parser disambiguates by position.

A **qualified name** like `core:string` or `ex:Dog` is **three tokens**: `Ident("core") Colon Ident("string")`. The parser stitches the three back into a single `QualifiedName` AST node. This is the same convention as EigenQL ([EigenQL §3.3](../eigenql/03-lexical-structure.md)) and it has the same consequence: the colon is also a standalone punctuation token (used in type annotations like `let x : T = ...`), so the parser disambiguates `core:string` from `let x : string` purely by what tokens precede the colon.

Qualified names resolve through namespace aliases declared with `namespace` (chapter 4 §4.1). A bare identifier in a position that expects an IRI either resolves through context (e.g., a component name resolves to a registered component IRI) or is a plain field name.

## 3.4. Literals

```
STRING ::= '"' (ESC | [^"\\])* '"'
ESC    ::= '\"' | '\\' | '\n' | '\r' | '\t'

INTEGER ::= '-'? [0-9]+
FLOAT   ::= '-'? [0-9]+ '.' [0-9]+ ([eE] ('+'|'-')? [0-9]+)?

BOOLEAN ::= 'true' | 'false'
```

String escapes are limited to the five forms above. Numbers may begin with a leading `-` (parsed as part of the literal); subtraction operators are not currently in the expression grammar.

## 3.5. Operators and punctuation

| Token | Meaning |
|---|---|
| `=` | Assignment in `let`, field bindings in `Construct`, namespace declarations |
| `->` | Function-type arrow, used in `program ... : T -> U` and codata observation types like `{j < i} -> Stream(A, j)` |
| `\` | Lambda introducer (ASCII), e.g. `\x -> e` |
| `λ` | Lambda introducer (Unicode, U+03BB), e.g. `λx -> e` |
| `.` | Property projection (`input.ex:name`) |
| `;` | Statement separator (between `let`s, between observations) |
| `:` | Type annotation, qualified-name separator, parent class in `class C : Parent` |
| `,` | List separator |
| `<` | Size bound in bounded binders (`{j < i}`, `{j : core:Size < i}`) |
| `(` `)` | Function call args, parameter telescopes |
| `{` `}` | Block delimiters: declaration bodies, expression blocks, bounded binders |
| `[` `]` | Reserved (currently unused in expressions) |

The two lambda forms `\` and `λ` are interchangeable. `\` is the ASCII escape hatch for keyboards without easy Unicode entry; `λ` is the canonical form.

## 3.6. End-of-input

The lexer always emits a trailing `Eof` token. Parsers that consume all tokens up to `Eof` finish cleanly; consumers that stop early can use the position carried by `Eof` for end-of-file diagnostics.

## 3.7. What the lexer does not do

The lexer is intentionally minimal:

- **No keyword resolution beyond the fixed table.** `Dog` is `Ident("Dog")`; whether it's a class or a variable name is decided later.
- **No qualified-name composition.** `core:string` is three tokens; the parser combines them.
- **No size-bound parsing.** The `<` token is emitted whenever it appears; the parser decides whether `<` is a size bound (inside `{...}`) or some other use.
- **No bracket matching.** Mismatched braces surface as parser errors, not lexer errors.

## 3.8. Comparison with EigenQL's lexer

For readers coming from the [EigenQL guide](../eigenql/03-lexical-structure.md), the differences are:

- ESL has **no `?` variable prefix** — variables and property names look identical to identifiers (parser disambiguates by position).
- ESL keywords are **lowercase** (except `Construct` and the sort names `Prop` / `Set` / `Type`); EigenQL keywords are **uppercase** (`MATCH`, `WHERE`, etc.).
- ESL has **two lambda forms** (`\` and `λ`); EigenQL has no anonymous functions.
- ESL has the **`<` token** for size bounds; EigenQL uses `<` only as a comparison operator.
- ESL has an explicit **`Construct` keyword**; EigenQL relies on `RETURN [Class] { ... }` for typed result construction.

---

Next: **[4. Declarations →](04-declarations.md)**
