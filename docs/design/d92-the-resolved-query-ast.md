# D92 — The resolved query AST

**Issue:** [#248](https://github.com/eigenius/eigenius/issues/248). **Written** `2026-09-14`, against `main` at `54cb647`.

## The gap

EigenQL names things two ways. `MATCH Notebook(?n) { title: ?t }` writes short names; `MATCH "urn:ex:Notebook"(?n) { "urn:ex:title": ?t }` writes IRIs. The AST holds both as `Name`, and a pass somewhere between parsing and evaluation is expected to turn the first into the second.

Nothing in the types says which passes do that, which positions they cover, or whether a position has been through one. The result is that after eight `Name` positions and four passes, **two positions are resolved once and written back, and six are not**:

| position | resolved at type-check | re-resolved at evaluation | written back |
|---|---|---|---|
| `PropertyPattern.property` | once, `resolve.rs:484` | no | **yes** |
| `Expression::DotPath.segments` | once, `resolve.rs:484` | no | **yes** |
| `Pattern.class` | **twice** — `type_check.rs:294` to validate, `resolve.rs:328` to build a dot-path scope | **yes**, `evaluate/pattern.rs:558` | no |
| `FiberClause.query_class` | **twice** — `type_check.rs:1170`, `resolve.rs:347` | — | no |
| `FiberClause.institution` | once | **yes**, `evaluate/fiber.rs:529` | no |
| `ParamBinding.name` | once, into a `short_to_iri` table | **yes**, `evaluate/fiber.rs:328` rebuilds the same table | no |
| `Query.result_classes` | **never** | consumed raw, `evaluate/mod.rs:223` | no |
| `ReturnItem.name` | **never** | consumed raw, `return_shape.rs:62` | no |

A pattern class short name is resolved **three times per query**, by three call sites, none of which keeps the answer. `resolve_scoped_name` enumerates the typed-resource IRIs for a metaclass and resolves each candidate to read its `short_name`, so this is not free — D2-scale chains pay it per query, per site.

**Every duplicate is a place where two mechanisms can disagree about one name**, and that is not hypothetical. It is what #249 fixed for property names: the type-checker matched a short name against a declared property's `core:short_name`, the evaluator matched it against the *local name* of whatever IRIs the resource happened to carry, and they disagreed in both directions with no diagnostic — a declared `urn:ex:title_text` with `short_name "title"` could not be named `title`, and an undeclared `urn:other:title` could.

**The two unresolved rows are a different failure.** `result_classes` and `ReturnItem.name` are not resolved by anything at all. They are consumed raw.

## What `Name` conflates

`ReturnItem.name` is not a chain reference. `return_shape.rs:62` reads it as:

```rust
let prop_iri = match &item.name {
    Name::FullIri(iri) => iri.clone(),
    Name::ShortName(s) => fp.row_property_iri(s),
};
```

`fp.row_property_iri(s)` **synthesises** a property IRI from the query fingerprint. `RETURN [] { total: SUM(?x) }` does not assert that anything called `total` is declared anywhere — it names a column in the output document.

So `Name` is carrying two unrelated things:

- **a reference** to a resource the chain must already declare (a class, a property, a query class, an institution), which resolution must turn into an `Iri` or reject;
- **a label** the query author invents for an output column, which nothing can resolve because there is nothing to resolve it against.

They are spelled the same and typed the same, so the discipline for one silently reads as the discipline for the other. Six positions not following the rule is the predictable outcome of a type that does not distinguish the rule's subjects.

## The decision: parameterise the AST over its name type

One set of AST structs, generic in the name:

```rust
pub struct Pattern<N> {
    pub subject: Variable,
    pub class: Option<N>,
    pub properties: Vec<PropertyPattern<N>>,
    pub negated: bool,
}

pub type ParsedProgram = Program<Name>;      // what the parser builds
pub type ResolvedProgram = Program<ClassRef>; // what everything after resolution sees
```

and resolution as a total function between the two:

```rust
pub fn resolve(program: ParsedProgram, layer: &Layer) -> Result<ResolvedProgram, Vec<QueryError>>
```

**The property that matters is not that the state is checked. It is that a missed position does not compile.** To produce a `Program<Iri>` the pass must produce an `Iri` for every parameterised position; there is no arm it can skip.

That is exactly how the FIBER defect happened. `resolve_part` matched `Clause::Pattern` and let `Clause::Fiber` fall through a `else { continue }`, so a dot-path inside a FIBER param kept its short names, type-checked with **zero errors**, and failed at evaluation with a message naming the resolution pass. Under a parameterised AST that `continue` is a type error: the function has no `Iri` to put in the `FiberClause` it skipped.

Labels are *not* parameterised. `ReturnItem.name` and any other output-column position take a distinct `ColumnLabel` type, so the two meanings stop sharing a spelling, and classifying each position becomes a step the compiler forces rather than a judgement someone has to remember to make.

### Not every resolved position is an `Iri`

A pattern class may name a `DEFINE` relation, which is not a chain resource:

```eigenql
DEFINE ancestor(?a, ?b) ...
MATCH ancestor(?x, ?y)
```

`check_match_part` exempts relation names from class resolution today, as a special case inside the check. Under the staged AST the resolved type states it:

```rust
pub enum ClassRef {
    Chain(Iri),        // a core:Class on the chain
    Relation(String),  // a DEFINE relation in this program
}
```

The exemption becomes a variant, and every consumer of a resolved class has to say which it handles. This is the kind of thing the parameterisation is for: the special case existed, undocumented in the types, in one checking function.

## Why not the alternatives

**Two hand-written AST types** (`ast::parsed::*` and `ast::resolved::*`) gets the same guarantee and duplicates roughly twenty structs, with every future field added twice. The parameterised form is the same idea without the copy.

**A newtype only the resolver can construct** (`ResolvedName(Iri)`) is smaller and does not get the guarantee. The parser still has to build *something* for each position, so either the position is `Option<ResolvedName>` — which is the current situation with extra ceremony — or it is two types anyway.

**Keeping the guards** is the status quo, and the audit above is what the status quo produces: eight positions, six wrong, and nothing in the types to tell them apart. #249 fixed two positions by noticing them. Noticing does not scale to the next contributor.

## The work

Blast radius is 12 files, all under `kernel/src/query/` except `program/embedder.rs` and `query/document.rs`.

1. **Split the label meaning out of `Name`.** `ReturnItem.name` becomes `ColumnLabel`. `Expression::Object`'s keys go with it — and the variant should be deleted rather than converted: the parser has no construction site for it and the evaluator errors on it, so it is unreachable today.
2. **Parameterise** `Program`, `Query`, `MatchPart`, `Clause`, `Pattern`, `PropertyPattern`, `Expression`, `FiberClause`, `ParamBinding`, `RuleDefinition` over `N`.
3. **Make `resolve` a pass**, `ParsedProgram -> Result<ResolvedProgram, Vec<QueryError>>`, absorbing all eight positions. The pipeline becomes lex → parse → stratify → **resolve** → type-check → evaluate.
4. **Type-check stops resolving.** It takes `&ResolvedProgram` and only checks. This also reverts the `&mut Program` signature #249 introduced, which was resolution wearing a checking pass's name.
5. **Evaluation takes `&ResolvedProgram`.** Delete `resolve_name` from `evaluate/pattern.rs`, the alias re-resolution in `evaluate/fiber.rs:529`, and the `short_to_iri` rebuild at `evaluate/fiber.rs:328`. Both guards from #248 go with them, having nothing left to guard.

Stages 1 and 2 are mechanical and large. Stages 3 to 5 are where the six positions actually change behaviour, and each should land with the same discipline #249 used: a test that fails against the unresolved version.

## Consequences

**Resolution errors will block type errors.** Today one pass reports both together. A staged pipeline cannot: without a `ResolvedProgram` there is nothing for type-check to run on. A query with a typo'd class and a genuine type error will report the typo first and the type error on the next run.

That is a real cost and the note takes it deliberately. A name that does not resolve makes every downstream check about it meaningless, so the second error is as likely to be noise as signal. The alternative — a partial program carrying resolution holes — reintroduces exactly the representable-but-invalid state this note exists to remove.

**Per-query resolution cost drops.** Three class resolutions become one, two query-class resolutions become one, and the FIBER param table is built once instead of twice. The #249 review measured 2.5s of type-check for one untyped five-property query on a 100-layer chain; that is the same `resolve_scoped_name` machinery, and this removes repeat calls rather than making any single call cheaper. Not the motivation, and worth measuring after.

**D2 needs a section.** The specification describes short-name resolution per construct (§5.4, §5.6.1, §5.8) and does not say that resolution is a pipeline stage with a before and an after. That is now a language-level fact, not an implementation detail.

## Open questions

1. **Does `Query.result_classes` resolve like a pattern class, or is it also a label?** It is stamped as `is_a` on result rows, which is a chain reference and should resolve. But nothing checks it today, so adding resolution may reject queries that currently run. This needs the same blast-radius measurement #249 did before the rule lands, not after.
2. **Should `ClassRef::Relation` carry the resolved `RuleDefinition` rather than its name?** It would remove a second lookup at evaluation, at the cost of a lifetime or an index in the AST.
3. **Is `stratify` before or after resolution?** It reads rule names and dependency edges, which are syntactic, so either works. Before is cheaper — a program that fails stratification never pays for resolution.

## Scope

This changes no query semantics. Every query that resolves today resolves to the same IRIs afterwards; what changes is how many times, and whether a position that was never resolved starts being checked — which open question 1 covers.

No ontology edit, so no manifest move and no reseed.
