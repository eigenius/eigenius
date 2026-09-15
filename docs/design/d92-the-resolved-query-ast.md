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

One set of AST structs, generic in the *stage* they belong to:

```rust
pub trait Stage {
    /// A `MATCH` pattern's class: a chain class or a DEFINE relation.
    type PatternClass: Debug + Clone + PartialEq;
    /// Every other reference — property key, dot-path segment, RETURN result class,
    /// FIBER institution, query class, param, comorphism.
    type Ref: Debug + Clone + PartialEq;
}

impl Stage for Parsed   { type PatternClass = Name;     type Ref = Name; }
impl Stage for Resolved { type PatternClass = ClassRef; type Ref = Iri;  }

pub struct Pattern<S: Stage = Parsed> {
    pub subject: Variable,
    pub class: Option<S::PatternClass>,
    pub properties: Vec<PropertyPattern<S>>,
    pub negated: bool,
}
```

and resolution as a total function between the two instantiations:

```rust
pub fn resolve(program: Program<Parsed>, layer: &Layer)
    -> Result<Program<Resolved>, Vec<QueryError>>
```

**Two associated types, not one type parameter.** The first draft of this note said
`Program<Name>` to `Program<ClassRef>`, which does not work: the positions do not resolve
alike. A pattern class may name a chain class *or* a `DEFINE` relation, so it resolves to
a `ClassRef`; every other reference resolves to the `Iri` of a declared resource. Forcing
one resolved type on both makes that type a sum, and a sum lets a property key hold a
`Relation` — representable and invalid, which is the state this note exists to remove,
relocated rather than removed. Implementation surfaced it; the design is corrected here.

They stop at two. Distinguishing a property `Iri` from a class `Iri` in the type — so a
property key could not hold a class — wants a newtype per metaclass. That is a larger
change than this note, and nothing here needs it.

`S` defaults to `Parsed`, so code working on parsed programs reads unchanged and the
migration is per-consumer rather than all at once.

**The property that matters is not that the state is checked. It is that a missed position does not compile.** To produce a `Program<Iri>` the pass must produce an `Iri` for every parameterised position; there is no arm it can skip.

That is exactly how the FIBER defect happened. `resolve_part` matched `Clause::Pattern` and let `Clause::Fiber` fall through a `else { continue }`, so a dot-path inside a FIBER param kept its short names, type-checked with **zero errors**, and failed at evaluation with a message naming the resolution pass. Under a parameterised AST that `continue` is a type error: the function has no `Iri` to put in the `FiberClause` it skipped.

Labels are *not* parameterised. `ReturnItem.name` and any other output-column position take a distinct `ColumnLabel` type, so the two meanings stop sharing a spelling, and classifying each position becomes a step the compiler forces rather than a judgement someone has to remember to make.

### Not every resolved position is an `Iri`

A pattern class may name a `DEFINE` relation, which is not a chain resource:

```eigenql
DEFINE ancestor(?a, ?b) ...
MATCH ancestor(?x, ?y)
```

`check_match_part` exempts relation names from class resolution today, as a special case inside the check. Under the staged AST the resolved type states it — and it is why `Stage` needs `PatternClass` separately from `Ref`:

```rust
pub enum ClassRef {
    Chain(Iri),           // a core:Class on the chain
    Relation(RelationId), // which DEFINE relation, by id
}
```

The exemption becomes a variant, and every consumer of a resolved class has to say which it handles. This is the kind of thing the parameterisation is for: the special case existed, undocumented in the types, in one checking function.

**`Relation` carries an id, not the definition and not the name.** Carrying the name leaves a string lookup at every use, which is the second-resolution pattern this note exists to remove. Carrying the `RuleDefinition` itself is impossible: `stratify` rejects only *negation* cycles, so ordinary positive recursion is legal Datalog — `ancestor(?a,?b) :- parent(?a,?c), ancestor(?c,?b)` — and a `ClassRef` embedding its own definition would be an infinite value for exactly the rules the feature exists for.

**The id identifies a RELATION, not a rule**, and an earlier draft of this note got that wrong by calling it "an index into `Program.definitions`". A relation may be defined by several rules:

```eigenql
DEFINE Reach(?t) FROM MATCH ?o { "urn:eigenius:t:thesis": ?t }
DEFINE Reach(?n) FROM MATCH Reach(?m) { "urn:eigenius:t:dep": [... ?n ...] }
```

— one relation with a base case and a recursive case. Indexing definitions gives those two ids, which splits the derived facts into two relations, and the closure then never accumulates. Implementation hit it: the reachability tests reported 2 unreachable nodes where 1 was right. `ast::relation_ids` assigns one id per distinct name, by first appearance.

## Why not the alternatives

**Two hand-written AST types** (`ast::parsed::*` and `ast::resolved::*`) gets the same guarantee and duplicates roughly twenty structs, with every future field added twice. The parameterised form is the same idea without the copy.

**A newtype only the resolver can construct** (`ResolvedName(Iri)`) is smaller and does not get the guarantee. The parser still has to build *something* for each position, so either the position is `Option<ResolvedName>` — which is the current situation with extra ceremony — or it is two types anyway.

**Keeping the guards** is the status quo, and the audit above is what the status quo produces: eight positions, six wrong, and nothing in the types to tell them apart. #249 fixed two positions by noticing them. Noticing does not scale to the next contributor.

## The work

Blast radius is 12 files, all under `kernel/src/query/` except `program/embedder.rs` and `query/document.rs`.

1. **Split the label meaning out of `Name`.** `ReturnItem.name` becomes `ColumnLabel`. `Expression::Object`'s keys go with it — and the variant should be deleted rather than converted: the parser has no construction site for it and the evaluator errors on it, so it is unreachable today.
2. **Parameterise** `Program`, `Query`, `MatchPart`, `Clause`, `Pattern`, `PropertyPattern`, `Expression`, `FiberClause`, `ParamBinding`, `RuleDefinition` over `N`.
3. **Make `resolve` a pass**, `ParsedProgram -> Result<ResolvedProgram, Vec<QueryError>>`, absorbing all eight positions and taking stratification's relation table so `RelationId` needs no second classification. The `ResolvedProgram` carries the strata forward, which removes the second `stratify` call at `evaluate/mod.rs:127`.
   - **`Query.result_classes` resolves like a pattern class.** It is stamped as `is_a` on every result row, so it is a chain reference and a query naming a class that does not exist should be told so. Nothing checks it today, so this is the one position in the note that can reject a query which currently runs: it needs the blast-radius measurement #249 used — the full workspace suite plus every EigenQL string in the notebooks, the TS clients and the notebook runtime — run *before* the tests are written, because the rule can only fail queries that previously passed.
4. **Type-check stops resolving.** It takes `&ResolvedProgram` and only checks. This also reverts the `&mut Program` signature #249 introduced, which was resolution wearing a checking pass's name.
5. **Evaluation takes `&ResolvedProgram`.** Delete `resolve_name` from `evaluate/pattern.rs`, the alias re-resolution in `evaluate/fiber.rs:529`, and the `short_to_iri` rebuild at `evaluate/fiber.rs:328`. Both guards from #248 go with them, having nothing left to guard.

Stages 1 and 2 are mechanical and large. Stages 3 to 5 are where the six positions actually change behaviour, and each should land with the same discipline #249 used: a test that fails against the unresolved version.

## A ninth position, knowingly outside the guarantee

`Expression::FunctionCall.name` is a bare `String`. A qualified call `inst:proc(?x)` is a
reference to a declared Decidable QueryClass, and it is resolved twice — `Iri::parse` plus
`index.query_class()` at type-check (`type_check.rs`), and the same pair again at
evaluation (`evaluate/expression.rs`).

It is not parameterised, so it sits outside the guarantee the argument above rests on: a
pass that forgot it would compile. Both mechanisms are identical today, so they cannot
disagree; what is lost is the compiler's help if one ever changes.

Stated here rather than left for a reader to notice, because "a missed position does not
compile" is the claim this note is built on and it is true of eight positions, not nine.
Closing it means the `name` field becoming a reference type the way the others did —
mechanical, and separable from this note's work.

## Consequences

**Resolution errors will block type errors.** Today one pass reports both together. A staged pipeline cannot: without a `ResolvedProgram` there is nothing for type-check to run on. A query with a typo'd class and a genuine type error will report the typo first and the type error on the next run.

That is a real cost and the note takes it deliberately. A name that does not resolve makes every downstream check about it meaningless, so the second error is as likely to be noise as signal. The alternative — a partial program carrying resolution holes — reintroduces exactly the representable-but-invalid state this note exists to remove.

**Per-query resolution cost drops, where the duplicates were.** Measured after, by
`kernel/tests/d92_resolution_cost.rs`, against `main` at `54cb647` on one machine. The
harness stops at type-check, so it sees two of `main`'s three class resolutions — the
third is at evaluation.

A chain of **2000 classes**, `MATCH Big(?x) { … }` with a short-name class:

| depth | main | branch | |
|---|---|---|---|
| 0 | 3ms | 2ms | |
| 25 | 10ms | 5ms | 2.0× |
| 50 | 18ms | 9ms | 2.0× |
| 100 | 34ms | 19ms | 1.8× |

A chain of **2000 properties** shows **no change at all** — 81ms against 86ms untyped at
depth 100, 1ms against 1ms typed. That is the honest shape of the result and it is worth
stating plainly: D92 de-duplicated *class* and *query-class* lookups, not property
lookups, so a chain with few classes cannot show a difference however many properties it
has. `resolve_scoped_name` is O(vocabulary of the metaclass × depth), and the metaclass
decides which chains benefit.

The GO bench is unmoved (cold 21-29ms, inside this branch's 19-27ms spread), which is what
a branch touching the whole evaluation path should show.

**D2 needs a section.** The specification describes short-name resolution per construct (§5.4, §5.6.1, §5.8) and does not say that resolution is a pipeline stage with a before and an after. That is now a language-level fact, not an implementation detail.

## Stratification runs first, and its answer is carried

`stratify` reads rule names and dependency edges. Both are syntactic — it needs nothing from the chain — so either order compiles. It goes **first**, for two reasons.

**A chain-independent failure should not pay a chain-dependent cost.** Resolution is where the expensive lookups live: `resolve_scoped_name` enumerates the typed-resource IRIs for a metaclass and resolves each candidate. A program with a negation cycle is malformed regardless of what any layer declares, and putting resolution first would make it pay for that before finding out.

**`ClassRef::Relation(RelationId)` needs the definition set fixed and indexed**, which is precisely what stratification already derives.

### The answer has to be carried, not recomputed

Stratification currently runs **twice per query**, and the first run's answer is thrown away:

```rust
// query/mod.rs:119 — computes Vec<Stratum>, keeps only the error
stratify::stratify(&program.definitions).map_err(|e| vec![e])?;

// evaluate/mod.rs:127 — computes it again
let strata = crate::query::stratify::stratify(&program.definitions)?;
```

Both calls are the same function, so unlike the `Name` positions they cannot disagree; this is waste rather than a correctness hazard. The set of relation names is separately rebuilt three times — `stratify.rs:40`, `type_check.rs:55`, and threaded into `resolve.rs:250` as a parameter.

This is the same shape as the rest of the note: a pass computes an answer, discards it, and a later pass computes it again. The staged pipeline is where that stops. Stratification's output — the strata order and the relation table `resolve` needs for `RelationId` — rides in the `ResolvedProgram` rather than being recomputed at evaluation.

That makes the pipeline lex → parse → **stratify** → **resolve** → type-check → evaluate, with each stage handing the next what it worked out rather than the next stage working it out again.

## Open questions

None outstanding. Three were opened by the first draft and all are decided above: `Query.result_classes` resolves as a chain reference; `ClassRef::Relation` carries an index rather than a name or an embedded definition; and stratification runs before resolution, with its result carried forward.

## Scope

This changes no query semantics for seven of the eight positions: every query that resolves today resolves to the same IRIs afterwards, and what changes is how many times. The exception is `Query.result_classes`, which nothing checks today and which will start rejecting a `RETURN` naming a class the chain does not declare. That is the one place this note can break a working query, and it is called out in the work above so the measurement happens first.

No ontology edit, so no manifest move and no reseed.
