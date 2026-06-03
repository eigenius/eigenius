# 3. Lexical structure

The lexer lives at [kernel/src/query/lexer.rs](../../../kernel/src/query/lexer.rs) and produces a stream of `Token` values from EigenQL source. This chapter lists every token kind and explains how they compose.

## 3.1. Comments and whitespace

- **Line comments**: `// ... \n`
- **Block comments**: `/* ... */` (not nested)
- Whitespace (spaces, tabs, newlines) is not significant beyond separating tokens.

Both are discarded by the lexer and never appear in the token stream.

## 3.2. Keywords

All keywords are matched **case-sensitively** and conventionally written in `UPPERCASE`. A bare identifier that happens to be lowercase (e.g. `match`) is not recognized as the `MATCH` keyword; it parses as an `Identifier`.

### Query structure

| Keyword | Purpose |
|---|---|
| `USING` | Import classes or declare institution aliases |
| `INSTITUTION` | Follows `USING` to declare an institution alias |
| `AS` | Binds a FIBER response to a variable, or an institution alias |
| `DEFINE` | Introduces a derived relation |
| `FROM` | Separates the `DEFINE` head from its body |
| `MATCH` | Structural pattern clause |
| `WHERE` | Boolean filter on bindings |
| `RETURN` | Shape of result rows |
| `FIBER` | Institution dispatch clause |

### Aggregation and ordering

| Keyword | Purpose |
|---|---|
| `GROUP` | Part of `GROUP BY` |
| `BY` | Part of `GROUP BY` / `ORDER BY` |
| `ORDER` | Part of `ORDER BY` |
| `ASC` / `DESC` | Sort direction |
| `DISTINCT` | Remove duplicate result rows |
| `LIMIT` | Truncate result set |
| `OFFSET` | Skip leading rows |

### Logical and set operators

| Keyword | Purpose |
|---|---|
| `AND` | Logical conjunction |
| `OR` | Logical disjunction |
| `NOT` | Logical negation, pattern negation, `NOT IN`, `NOT LIKE`, `NOT EXISTS` |
| `IN` | Set membership (right side is an array) |
| `LIKE` | SQL-style string pattern match |
| `EXISTS` | Binding-presence check (used as `NOT EXISTS(?var)`) |

### Built-in functions

These identifiers are reserved as keywords because the parser dispatches on them directly.

**Scalar**: `DATE`, `TIMESTAMP`, `REGEX`, `LENGTH`, `CONTAINS`, `CONCAT`

**Aggregate**: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`

Behaviour details are in [chapter 7 §7.7](07-expressions.md) and [chapter 13](13-appendix.md).

## 3.3. Identifiers and qualified names

An **identifier** begins with an ASCII letter or underscore and continues with letters, digits, underscores, or hyphens:

```
ident ::= [A-Za-z_] [A-Za-z0-9_-]*
```

An identifier that is not a keyword tokenizes as `TokenKind::Identifier(String)`.

**Qualified names** like `cap:within_tolerance` are **not a single token** — the lexer produces three tokens: `Identifier("cap")`, `Colon`, `Identifier("within_tolerance")`. The parser reassembles them in two places:

1. **Class references in `MATCH`**: `ShortName` / `FullIri` resolution via namespaces imported through `USING`.
2. **Function calls** (Phase 11e.2): `ns:local(args)` in expression position. See [chapter 7 §7.7](07-expressions.md) for how `parse_primary_expr` reads the two identifiers around a colon and stitches them back into `"ns:local"`.

## 3.4. Variables

A variable token is `?` followed by an identifier. The `?` prefix distinguishes query variables from identifiers and keywords.

```
variable ::= '?' ident
```

Stored as `TokenKind::Variable(String)` with the `?` prefix stripped. Variables carry bindings through the pipeline: patterns assign values to them, `WHERE` and `RETURN` reference them. A variable is **bound** if it appears as the subject of a non-negated `MATCH`, as the target of a property pattern, or as a `FIBER` clause's `AS ?var`. Referencing an unbound variable in `WHERE` / `RETURN` / `GROUP BY` / `ORDER BY` raises a type-check error.

## 3.5. Literals

### Strings

Double-quoted, supports the following escapes:

| Escape | Character |
|---|---|
| `\"` | literal `"` |
| `\\` | literal `\` |
| `\n` | newline |
| `\r` | carriage return |
| `\t` | tab |
| `\uXXXX` | Unicode code point (4 hex digits) |

Example:
```eigenql
"Hello, \"world\""
"urn:eigenius:core:Class"
```

**Note**: String literals can also appear as **IRI references** in positions expecting a `Name` (e.g. `MATCH "urn:..."(?x) { ... }`). The parser accepts either a bare identifier (`Name::ShortName`) or a string literal (`Name::FullIri`) at those positions.

### Integers

```
integer ::= '-'? [0-9]+
```

Negative sign is part of the token (not a unary minus). Range: `i64` (−9223372036854775808 to 9223372036854775807).

### Floats

```
float ::= '-'? ( [0-9]+ '.' [0-9]+ ) | ( [0-9]+ ('e' | 'E') '-'? [0-9]+ )
```

Examples: `3.14`, `1e10`, `-2.5`, `1.5e-3`. Parsed as `f64`.

### Booleans

```
boolean ::= 'true' | 'false'
```

These are **lowercase keywords** — case-sensitive. `TRUE` would lex as an identifier.

## 3.6. Operators and punctuation

### Arithmetic

| Token | Operator |
|---|---|
| `+` | Addition |
| `-` | Subtraction (when used between expressions) |
| `*` | Multiplication |
| `/` | Division |
| `%` | Modulo |
| `**` | Exponentiation |

Note: `-` is ambiguous in isolation — an integer literal `-5` is lexed as a single `NumberInt(-5)` token, but `?x - ?y` tokenizes `?x`, `-`, `?y`.

### Comparison

| Token | Operator |
|---|---|
| `=` | Equality |
| `<>` | Inequality |
| `<` | Less than |
| `<=` | Less or equal |
| `>` | Greater than |
| `>=` | Greater or equal |

### String

| Token | Operator |
|---|---|
| `\|\|` | String concatenation |

### Delimiters

| Token | Use |
|---|---|
| `(` `)` | Expression grouping, function application, pattern subject parens |
| `{` `}` | Pattern property list, RETURN object, DEFINE body |
| `[` `]` | Array literal, RETURN class tag list |
| `,` | Separator in lists |
| `.` | Property path separator (`?x.breed.name`) |
| `:` | Property binding in patterns, `ns:local` qualified names |
| `;` | Not used in the current grammar |

## 3.7. Complete token type (quick reference)

| Category | TokenKind variants |
|---|---|
| Keywords | `Match`, `Where`, `Return`, `Using`, `As`, `Define`, `From`, `And`, `Or`, `Not`, `In`, `Like`, `Exists`, `Group`, `By`, `Order`, `Asc`, `Desc`, `Distinct`, `Limit`, `Offset`, `Fiber`, `Institution` |
| Scalar fns | `DateFn`, `TimestampFn`, `RegexFn`, `LengthFn`, `ContainsFn`, `ConcatFn` |
| Aggregate fns | `CountFn`, `SumFn`, `AvgFn`, `MinFn`, `MaxFn` |
| Literals | `StringLit`, `NumberInt`, `NumberFloat`, `BooleanLit` |
| Ident / var | `Identifier`, `Variable` |
| Compare | `Eq`, `Neq`, `Lt`, `Lte`, `Gt`, `Gte` |
| Arith | `Plus`, `Minus`, `Star`, `Slash`, `Percent`, `DoubleStar` |
| String | `Pipe2` |
| Delim | `LParen`, `RParen`, `LBrace`, `RBrace`, `LBracket`, `RBracket`, `Colon`, `Comma`, `Dot` |
| EOF | `Eof` |

---

Next: **[4. Program structure →](04-program-structure.md)**
