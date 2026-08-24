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
namespace  class  property  resource  program  data  axiom  def  macro
merge_comorphism  text_index  vector_index
```

**Expression keywords** appear inside `program` bodies:

```
let  case  match  returning  Construct  map  reduce  lambda
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
IDENT  ::= [a-zA-Z_] [a-zA-Z0-9_]*
QUOTED ::= "'" [a-zA-Z0-9_-]+ "'"
```

A bare identifier (`Dog`, `name`, `short_name`) is one `Ident` token. There's no distinction at the lexer level between class names, property names, variable names, and component names — the parser disambiguates by position.

A **qualified name** like `core:string` or `ex:Dog` is **one `QualName` token**, not three. The lexer forms it when an identifier is immediately followed — no whitespace — by `:` and another name segment. That is what frees a space-surrounded `:` to mean the binder / annotation colon, so `base : ex:Nat` (`Ident Colon QualName`) and `ex:Nat` (`QualName`) are different token streams rather than a spacing convention.

### Quoted identifiers

A name the bare form cannot spell is written between single quotes:

```esl
resource ex:'obo-foundry' : core:Class { … }   // hyphen
resource ex:'14e82c39' : core:Class { … }      // leading digit (content hashes)
resource 'program':Foo : core:Class { … }      // keyword in the namespace half
```

The quotes are **lexical only** — `ex:'obo-foundry'` denotes exactly `urn:eigenius:ex:obo-foundry`. Either half of a qualified name may be quoted.

**The charset is `[A-Za-z0-9_-]` and `#` is excluded deliberately.** Fourteen families of kernel-minted binder name — `TC#`, `G#`, `CB#`, `IDX#`, `IH#`, `A#`, `HB#`, `HA#`, `HM#`, `HV#`, `AR#`, `ADV#`, `DIST#`, `AGG#` — are collision-free *because* `#` cannot occur in an ESL identifier; the recursor and iota reduction both give that as their argument. A quote admitting arbitrary text would make those names forgeable, and the failure would show up as an eliminator capturing a user-written name, which the type checker would accept. The restriction costs nothing: no chain IRI contains `#`, and across every shipped ontology the only unspellable shapes are the hyphen and the leading digit.

`eigenius decompile` **quotes minimally** — a name that lexes bare is printed bare, so quotes appear only where they are load-bearing.

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
| `->` | Function-type arrow, used in `program ... : T -> U` and constructor types like `A -> ex:List(A)` |
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
