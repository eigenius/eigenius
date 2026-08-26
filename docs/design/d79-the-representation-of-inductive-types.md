# D79 — The representation of inductive types

**Status: design.** Completes the inductive-type work D76 began. Split out of
[D77](d77-merge-as-a-pushout-of-environments.md) on `2026-08-25`, which had accumulated three
separable projects; this is the one that is buildable now and is a prerequisite for the other two.

D76 made the layer chain the typing environment: the chain binds **names to declarations**, and
`Env::lookup` returns one `InductiveDecl`. This document finishes the representation half of that
change. Three defects, each found by a consumer that needed something the representation does not
provide:

| # | defect | found by |
|---|---|---|
| 1 | three declarations carry D47-encoded terms, and one of them silently no-ops | D77 §3 needing to enumerate term dependents |
| 2 | an encoded term contributes no triples, so nothing can query what it mentions | D76 Phase A, forced to hand-write a second reference walker |
| 3 | an inductive constructor is redefinable, so every term mentioning it can silently change meaning | the cost analysis for fixing #2 |

**Not covered: merge.** D77 motivated this work and consumes it, but nothing here is merge-specific —
#1 and #2 are wrong on the linear commit path too, and #3 is a soundness rule that stands on its own.

---

## 1. What D76 changed, and what it left

### 1.1 The chain binds names to declarations

Before D76, a layer was a store of resource **shapes**: "`R` depends on `i`" meant `R`'s property
graph points at `i` — an `is_a` target, a property value, a property key. The triple index is built
from exactly that (`extract_indexable_triples`, `layer/index.rs:294`), and every consumer that asks
"what depends on `i`" walks it.

After D76 the chain also binds **names to declarations**, and a resource carrying a proposition was
type-checked in that environment.

### 1.2 So a term is a dependency, and nothing can see it

"`R` depends on `i`" now also means **`R`'s term mentions `i`** — a `ConstRef` inside an encoded
proposition, an inductive named in a `ctor_type`, an axiom cited in a justification.

| dependency | how `R` reaches `i` | queryable today |
|---|---|---|
| **shape** | `is_a`, property value, property key | yes — the triple index |
| **term** | a `ConstRef` inside an encoded term | **no** |

Encoded terms live in `Value::Json`, and `extract_indexable_triples` emits triples only for
`Value::ResourceRef` under `resource` / `resource_array` predicates (`index.rs:306-340`). An encoded
term contributes **no triples at all**.

**The tree has already been bitten by this.** D76 Phase A needed its own reference walker for
`declaration_order`, documented at `layer/declaration_order.rs:113`:

> *"Descends into `Value::Json`, which the walker in `layer::supporting` does not. That one documents
> JSON as never carrying typed-reference semantics, which is true for its purpose and false here: an
> inductive's constructor argument types are stored as D47-encoded JSON, so a walker that skips `Json`
> finds **no inductive-to-inductive edges at all**. Reusing it would produce an empty graph for
> precisely the case `OrderError::MutualInductives` exists to catch, **and would look like it
> worked**."*

Two consumers, two hand-rolled descents, and the second was written *because the first was unusable*.
That is the signal to fix the representation rather than write a third walker.

---

## 2. The three corrections

### 2.1 The data type exists; three declarations are in use for it

`core:inductive` is a declared `core:DataType` whose `class_types` must name exactly one
`InductiveType` (D32 §3.5). It is not opaque to the kernel: Rule 16 walks the tagged-dict tree against
the type's ctors and `arg_types`, and Rule 21 (`check_type_expr_well_typed`) decodes and
**type-checks** every `eigentt:TypeExpr`-ranged value, rejecting unresolved `ConstRef`s
(`validation/rules/inductive.rs:171`, `validation/rules/eigentt_value.rs`).

Three different declarations carry D47-encoded terms today:

| declaration | properties | validated | indexed |
|---|---|---|---|
| `core:inductive` + `class_types eigentt:TypeExpr` | `eigentt:axiom_statement`, `definition_type`, `definition_body` | Rules 16 + 21 | no — and says so |
| `core:resource` + `class_types eigentt:TypeExpr` | `lexicon:cat`, `lexicon:sem_type`, `reflection:canonical_proposition`, `core:type_name` | Rule 21 | **claims to be, isn't** |
| `core:json` | `core:ctor_type` | **nothing** | no |

Row 1 is honest — `core:inductive`'s own description says *"the wire shape is opaque JSON to
surrounding chain queries."* That sentence was a decision, and this document is the case against it.

**Row 2 is the defect.** Declaring a term-valued property `core:resource` makes both reference
consumers accept it and then do nothing:

- `extract_indexable_triples`' `wk::RESOURCE` arm matches `Value::String | Value::ResourceRef` and
  drops `Value::Json` into `_ => {}` (`index.rs:321-328`) — **zero triples**;
- Rule 22(b)'s filter `dt != RESOURCE && dt != RESOURCE_ARRAY` **passes**, then `iris_of` calls
  `as_iri()`, which returns `None` for `Value::Json` (`resource.rs:140`) — **zero checks**.

Neither errors. `lexicon:cat` and `lexicon:sem_type` are declared reference-typed on every lexicon
entry and contribute nothing to either mechanism. Nothing is *unchecked* — Rule 21 covers those slots
— but not by the mechanism the declaration names, which is the same "looks like it worked" shape as
the `declaration_order` walker.

Row 3 is what forced that walker: `ctor_type` is `core:json`, so no rule validates it.

**Decision: normalise rows 2 and 3 onto row 1.** `lexicon:cat`, `lexicon:sem_type`,
`reflection:canonical_proposition`, `core:type_name`, `core:ctor_type` become `core:inductive` +
`class_types eigentt:TypeExpr`. This is a bootstrap-ontology edit and therefore requires a reseed.

### 2.2 The indexer arm

**No new `Value` variant is needed.** Indexability is decided by the *predicate's declared data type*,
not the value's variant — `extract_indexable_triples` already resolves `prop_def.data_type` per
predicate. `core:json` and `core:inductive` share `Value::Json` on the wire and the predicate
distinguishes them.

So: a `wk::INDUCTIVE` arm that harvests the term's `ConstRef` IRIs, deduplicated per subject, emitted
under a synthetic **`core:mentions`** predicate rather than the carrying property. The question the
merge pass asks is *"which resources mention declaration `i`"*, not *"which mention it in slot
`cat`"*, so one predicate answers it in a single `scan_predicate_object(core:mentions, i)` range.

Consequences: §1.2's term-dependent enumeration becomes an index lookup rather than a chain walk, and
`declaration_order`'s bespoke walker can be deleted in favour of the same extraction.

### 2.3 Sealing the constructor vocabulary

Indexing every mention naively is expensive at lexicon scale, and expensive in a specific,
diagnosable way. A lexicon entry's `cat` is `cat_np(wn:n00001740, num_sg)`; multiplied over a
7.6M-entry chain, the posting list for `lexicon:cat_np` has millions of members. It is not *wrong* —
rebinding `cat_np` genuinely would invalidate every entry — but nothing can rebind `cat_np`, and
nothing should be able to.

**The rule: an `InductiveType` or one of its constructors may not be redefined.** D76 made the reason
literal — the chain binds names to declarations and `Env::lookup` returns *one* `InductiveDecl`. For a
class, "add a parent" is a monotone edit that the alignment layers depend on. For an inductive there
is no monotone edit: changing constructors changes the type, and every committed term mentioning it
silently means something else.

Two static scans over the repo's ontologies, experiments and demo files (approximate — ESL
constructor extraction undercounts bare-enum arms):

| candidate rule | scope | files that violate it today |
|---|---|---|
| seal **all** bootstrap IRIs (~1211) | 21 compiled-in ontologies | **6** |
| seal **inductive types + constructors** (193 repo-wide) | the term vocabulary | **0** |

The blanket rule is too broad, and the six say why. Five are parsing probes redefining closed-class
*entries* (`resource lexicon:among_prep : lexicon:LexicalEntry`) — instances. The sixth is
`ontologies/encoding/claim-kind-alignment.esl`, which is chain-loaded in the demo and redeclares
`enc:Finding` / `Observation` / `Classification` with lexicon parents added; its header names the
idiom as *"the layered-resolution pattern the wordnet↔umls alignment established."* Sealing all of
bootstrap breaks that. Sealing the constructor vocabulary costs nothing — zero violations across the
whole tree — which is the right time to impose a rule.

**What the seal buys the index.** Mentions whose object is sealed are dropped: they can never enter a
`rebound` set, so their posting lists can never be queried. In `cat_np(wn:n00001740, num_sg)` that
removes two of three. `wn:n00001740` survives — and must, because the alignment layer redefines
WordNet sense classes, which is exactly the live merge hazard. Per-entry mentions fall from ~5 to
~1-2, and every retained edge is one a merge can actually break.

**What the seal does not do.** An earlier draft cited `layer/witness_index.rs:1133` — where witness
credit survives redefinition of a class a proposition quantifies over — as evidence that the seal has
standing beyond merge, and said sealing "removes it for the constructor vocabulary." That reads as if
the seal addresses the cited case. It does not: the rebound name in that test is `Dog`, a **class**,
and classes stay redefinable by design (§5). The seal covers only the sub-case where the rebound name
is a constructor, which is not what that test exhibits.

The witness index is a real defect and a closely related one, but it is related as a *sibling*, not
as something the seal fixes. D80 §2 states the relationship.

**It is not a substitute for the pass.** #225's rebound IRI is a domain class, and domain classes are
unsealed by construction — that is where modeling happens. The seal shrinks the index and removes the
largest posting lists; it does not remove the need for §3's rebound-set pass.


---

## 3. Phases

Each phase begins with a code audit, per the discipline D76's phases established — every one of its
seven audits corrected something the design had asserted.

- **P1 — the seal** (§2.3). A validation rule refusing a layer that redefines an `InductiveType` or
  one of its constructors. Measured at zero violations, so it lands before anything depends on it.
  Gate: the rule fires on a hand-built violating layer; the full workspace suite, the demo and the
  parse gate are unperturbed. **No reseed** — it adds a rule, not an ontology edit.
- **P2 — the declarations** (§2.1). `lexicon:cat`, `lexicon:sem_type`,
  `reflection:canonical_proposition`, `core:type_name`, `core:ctor_type` become `core:inductive` +
  `class_types eigentt:TypeExpr`. **Bootstrap edit, so a reseed**; batch with any other pending
  bootstrap change. Gate: reseed completes at the current resource count with 0 errors, and
  `ctor_type` now reaches Rules 16/21 — asserted by a malformed `ctor_type` that previously loaded
  and now does not.
- **P3 — the indexer arm** (§2.2). `wk::INDUCTIVE` in `extract_indexable_triples`, emitting deduped
  `core:mentions` triples and skipping sealed objects. Gate: index growth on the lexicon chain is
  **measured, not estimated**, against the ~1-2-per-entry prediction; `declaration_order`'s bespoke
  walker is deleted and its `MutualInductives` tests still pass on the shared extraction.

**Order is not arbitrary.** The seal first, so the indexer can rely on it; the declarations next, so
the indexer has a `core:inductive` predicate to match; the arm last. P2 is the only reseed.

---

## 4. What this unblocks

- **[D80](d80-witness-and-institution-machinery.md)** — binding-aware witness lookup needs the name
  set of an attested proposition, which becomes one `core:mentions` range query.
- **[D77](d77-merge-as-a-pushout-of-environments.md)** — merge's term-dependent enumeration is the
  same query. Without §2.2 it would be a third hand-rolled full-chain walker.
- **The linear commit path.** Rule 22 currently checks nothing for term-valued slots declared
  `core:resource` (§2.1); after P2 they are `core:inductive` and Rule 21 owns them unambiguously.

## 5. What this does not cover

- **Classes and properties stay redefinable.** §2.3 seals inductive types and constructors only.
  Redefining a class to add parents is a load-bearing modeling idiom — the wordnet-umls alignment and
  `claim-kind-alignment.esl` both rely on it. Sealing more would trade a checkable hazard for a
  blocked workflow, and checking that hazard is D77's subject.
- **Genuinely opaque JSON stays `core:json`.** The Julia solver payloads (`primal_solution_kv`,
  `witness_data`, `trajectory_u`) are institution-interpreted blobs with no typed-reference
  semantics. §2.1 moves only the properties that carry D47-encoded terms.
- **Proposition identity.** Unchanged and environment-blind; that is D80's subject.

## 6. References

- D76 (the typing environment; Phase A is where the second walker was written)
- D32 §3.5 (`core:inductive` and singleton `class_types`), D47 §5 (`ConstRef` resolution)
- D77 §3 (the consumer that found defect #1), D80 (the consumer that finds it on the linear path)
- `layer/index.rs` (`extract_indexable_triples`), `layer/declaration_order.rs`,
  `validation/rules/inductive.rs`, `validation/rules/eigentt_value.rs`,
  `validation/rules/reference_integrity.rs`
