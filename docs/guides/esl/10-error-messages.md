# 10. Error messages

ESL errors come from three pipeline stages — lexer, parser, compiler — plus a fourth tier of errors from the kernel itself when type-checking or evaluating compiled programs. The structured error type is [`EslError`](../../../kernel/src/esl/error.rs):

```rust
pub struct EslError {
    pub position: Option<Position>,
    pub phase: EslPhase,    // Lexer | Parser | Compiler
    pub message: String,
}
```

A typical error renders as `<line>:<col>: [<Phase>] <message>`.

This chapter walks the common errors per phase, with the verbatim message you'll see (or close to it), what it means, and how to fix it.

## 10.1. Lexer errors

The lexer is small and rejects very few inputs. The common ones:

**Unterminated string literal**

```
3:14: [Lexer] unterminated string literal
```

You opened `"` but never closed it. Add the closing `"`. Multi-line string literals are not supported — the line break is permitted inside the string content, but the close quote must appear.

**Invalid escape**

```
2:22: [Lexer] invalid escape: '\q'
```

ESL recognises only `\"`, `\\`, `\n`, `\r`, `\t`. Use one of those, or write the character literally.

**Unexpected character**

```
1:5: [Lexer] unexpected character: '@'
```

The character isn't part of any ESL token. Check for stray punctuation or typos. Note that `?` is not a valid character in ESL (unlike EigenQL where `?` introduces variables).

**Unexpected `-`**

```
1:9: [Lexer] unexpected '-'
```

A `-` not followed by `>` (for the arrow `->`) or a digit (for a negative number) is a lexer error. ESL has no infix subtraction operator at the surface level.

## 10.2. Parser errors

Parser errors are the most common in practice because the surface grammar is strict about clause and block delimiters.

**Expected token**

```
5:12: [Parser] expected '{' but found Ident("retirement")
```

The parser was expecting a specific token shape. Frequent causes:

- Forgot a `{` opening a class body, property body, program body, or `data`/`codata` block.
- Missing `;` between `let` bindings or between observations.
- Missing `,` in a comma list (parameters, ctor args, recommends list, etc.).
- Wrong keyword case — `Construct` is title-case, `class`/`property`/etc. are lowercase.

**Unexpected end of input**

```
[Parser] unexpected end of input; expected '}'
```

Source ran out before a structural token closed. Usually a missing `}`. Check brace balance.

**Expected expression**

```
12:8: [Parser] expected expression, got RBrace
```

The parser was in expression position and got something that can't start an expression. Often a stray `;` or an early `}`.

**Bounded binder syntax**

```
8:14: [Parser] expected '<' or '}' in bounded binder
```

Inside a `{j ... }` binder, the parser expected either a kind annotation (`{j : Size}`), a bound (`{j < i}`), or both. Check the syntax — it must match one of the three documented forms in [§4.5](04-declarations.md).

## 10.3. Compiler errors

The compiler walks the AST, resolves names, and emits Eigon-JSON resources. Most of its errors are about resolution.

**Unknown namespace alias**

```
4:9: [Compiler] unknown namespace alias: 'ex'
```

You used `ex:Foo` but never declared `namespace ex = "...";` in this file. Each ESL file must declare every namespace it uses.

**Invalid IRI**

```
2:11: [Compiler] invalid IRI 'urn:bad space': URI scheme contains a space
```

The expanded IRI failed parsing. Check the namespace base URI and the local name for invalid characters.

**Bare name has no namespace**

```
[Compiler] bare name 'X' has no namespace and no in-scope binding
```

A bare identifier in a position that expects a qualified name was used without a binding. Usually means a class or component name was expected; either declare it locally (with a `data`/`codata`/`class`) or qualify it with `ns:X`.

**Field name needs namespace qualifier**

```
[Compiler] field name 'name' needs a namespace qualifier
```

In a `Construct` or `resource` block, field keys must be qualified property names (`ex:name`), not bare identifiers — bare identifiers would be ambiguous because property IRIs aren't local to a class.

**Constructor cannot take a configuration block**

```
[Compiler] constructor `succ` cannot take a configuration block — constructors are pure data
```

Constructor application doesn't accept the trailing `{ ... }` config form — that's reserved for component dispatch. If you wanted to construct a record/resource with named fields, use `Construct ex:Name { ... }` instead.

**Comorphism arity**

```
[Compiler] comorphism `urn:eigenius:institutions:dock_to_assay` expects exactly 1 source argument, got 2 positional arg(s)
```

Comorphisms take one source resource and nothing else. Pre-package additional info into an embedded resource and pass that.

**Decide predicate config block**

```
[Compiler] decide predicate `urn:eigenius:test:within_tolerance` does not accept a configuration block
```

Decide predicates take only positional args. Move any configuration into the args themselves or into the institution's setup.

**Function-call arity for components**

```
[Compiler] function `CompleteText` called with 3 positional arguments; non-constructor calls accept 1 (with optional config block) or 2 (legacy sugar). Multi-positional-arg dispatch is only defined for declared inductive constructors.
```

Components take exactly one positional input, optionally with a trailing config block, *or* the legacy two-positional form (`f(input, config)`). For more arguments, use a constructor application or wrap them in a structured resource.

## 10.4. Kernel errors (compile succeeded, kernel rejects)

Once the ESL compiler emits resources successfully, the resources can still fail when the kernel type-checks or evaluates them. These errors come from [`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs) and [`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs).

**Cannot infer type**

```
[Check] cannot infer type of expression — add an annotation or a returning clause
```

Type inference can't proceed without more information. Common cases:

- A `match` expression in a position without context (e.g., as the right-hand side of an unannotated `let`). Add a `returning T` clause.
- A lambda used outside a checking-mode position. Annotate the surrounding `let` with the function type.

**Class not found in layer chain**

```
[Eval] class 'urn:eigenius:example:Document' not found in layer chain
```

The kernel tried to resolve an `EigonClass(iri)` but the IRI isn't in the layer. Either:

- The class was never loaded into the layer (separate compilation issue — see [chapter 6](06-resources-types-and-the-layer.md)).
- The IRI is misspelled — check the namespace base URI and the local name match the declaration.

**Property not found**

```
[Eval] property 'urn:eigenius:example:nmae' not found
```

A property IRI used in a class's `requires`/`recommends` (or in a `Construct` field) isn't in the layer. Usually a typo — check the property declaration.

**Sized constructor: argument not strictly below upper bound**

```
[Check] InductiveCtor 'SizedNat.succ': size argument 'i' is not strictly below upper bound 'i'
```

You're applying a sized constructor with a size argument that doesn't satisfy the bounded-binder constraint. The constructor's signature requires `j < i`; you supplied `i` directly. Provide a strictly smaller size variable, or use the unbounded form if termination doesn't matter (which is rare).

**Recursive call not on a smaller size**

```
[Check] recursive call must be at a strictly smaller size
```

In a `match` arm body, you tried to recurse on a value of the same or larger size as the scrutinee. The TSO solver couldn't prove `j < i`. Restructure so the recursion is on a structurally smaller value (typically a constructor argument captured by the match).

**Type mismatch**

```
[Check] expected type 'Stream(A)' but got 'List(A)'
```

A normal type-check failure. The expected and actual types differ; the messages identify both. Walk back from the position to find which expression's type doesn't fit.

**FIBER requires institution registry**

This is an EigenQL error, not ESL — but you might see it if you compile ESL programs that use institution capabilities and then run them with a `Pure` or `Read` capability mode that lacks the registry. Use `EvalCtx::Check` or `EvalCtx::IO` with the registry attached. See [chapter 8](08-capability-modes.md).

**Comorphism unimplemented**

```
[Eval] institution does not implement `translate` for comorphism `urn:...`
```

The institution declared the comorphism in its `comorphism_types` but didn't override `translate` to actually perform the translation. This is a programmer error on the institution-author side — file an issue with the institution maintainer.

**Unknown function**

```
[Eval] unknown function: cap:something
```

Three causes:

1. The qualified name's IRI isn't registered as a decide predicate, comorphism, or component.
2. The institution registered but the specific procedure IRI wasn't included in `decide_procedures` or `comorphism_types`.
3. Typo in the name.

## 10.5. Debugging flow

When an ESL source rejects, work through the phases:

1. **Lexer error?** Fix the source character/quote/escape; re-try.
2. **Parser error?** Check brace/semicolon/comma balance and clause syntax around the position.
3. **Compiler error?** Check namespace declarations, IRI spellings, and form-specific arity rules.
4. **Kernel error after compile?** The compiled resources are valid Eigon-JSON, but the type-checker or evaluator rejected them. Read the message — it'll tell you whether the issue is type-mismatch, unresolved reference, or capability-mode mismatch.

For "this should work but doesn't" cases:

- Strip the program down to the minimal failing case.
- Check that every namespace your file uses is declared at the top.
- Check that every class/property/component you reference is loaded into the same layer the kernel sees.
- For institution capabilities: check that the registry was passed (`compile_with_institutions` rather than `compile`) and that the runtime mode includes the registry.

---

Next: **[11. Appendix →](11-appendix.md)**
