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
| 4 | every constructor payload carries a vestigial `@id` that reads as chain-resident openness | asking what a constructor's identifier is for |
| 5 | the recursor's motive codomain is a hard-coded `Sort(2)`, capping large elimination at `Set` ([#228](https://github.com/eigenius/eigenius/issues/228)) | its stated gate lifting when D76 Phase E2 landed |
| 6 | qualified constructor references do not parse, so same-named ctors cannot be disambiguated ([#24](https://github.com/eigenius/eigenius/issues/24)) | reading `resolve_ctor_iri`'s own error message |

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

### 2.1 The data type exists; almost nothing uses it

`core:inductive` is a declared `core:DataType` whose `class_types` must name exactly one
`InductiveType` (D32 §3.5). It is not opaque to the kernel: Rule 16 walks the tagged-dict tree against
the type's ctors and `arg_types`, and Rule 21 (`check_type_expr_well_typed`) decodes and
**type-checks** every `eigentt:TypeExpr`-ranged value, rejecting unresolved `ConstRef`s
(`validation/rules/inductive.rs:171`, `validation/rules/eigentt_value.rs`).

**The misdeclaration is systemic, not a few strays.** A full scan of every property whose
`class_types` names one of the 43 `InductiveType` declarations:

| declaration | count | validated | indexed |
|---|---|---|---|
| `core:inductive` | **5** | Rules 16 + 21 | no — and says so |
| `core:resource` | **23** | Rule 21 | **claims to be, isn't** |
| `core:resource_array` | **1** | Rule 21 | **claims to be, isn't** |
| `core:json` (no `class_types`) | 1 — `core:ctor_type` | **nothing** | no |

Correct today: `eigentt:axiom_statement`, `definition_type`, `definition_body`,
`formulas:operator_signature`, `lean:proposition`. Everything else reaches for `core:resource` —
`core:resource` is the default and `core:inductive` is the exception, which is why an earlier draft of
this section, working from a sample, reported four misdeclared properties rather than twenty-four:

| namespace | misdeclared properties |
|---|---|
| `lexicon` | `cat`, `sem_type`, `term`, `prop` |
| `reasoning` | `proposition`, `candidate_proposition`, `certificate`, `justification` |
| `stats` | `sample_set_value`, `effect_size`, `variance_assumption`, `directionality`, `outlier_exclusion`, `multiple_comparison_correction`, `autocorrelation_structure` |
| `objective` | `proposition`, `option_claim`, `selected`, `options` *(the one `resource_array`)* |
| `core` | `type_name`, `param_kind`, `result_sort` |
| `reflection` / `enc` | `canonical_proposition`, `antecedent_term` |

Note the range targets are not only `eigentt:TypeExpr`: `core:result_sort` ranges at `core:Level`,
`objective:selected` / `options` at `core:Option`, and the seven `stats` properties at their own
institution inductives. The defect is about the *declared data type*, not about which inductive.

The `core:inductive` row is honest — its own description says *"the wire shape is opaque JSON to
surrounding chain queries."* That sentence was a decision, and this document is the case against it.

**The `core:resource` rows are the defect.** Declaring a term-valued property `core:resource` makes
both reference consumers accept it and then do nothing:

- `extract_indexable_triples`' `wk::RESOURCE` arm matches `Value::String | Value::ResourceRef` and
  drops `Value::Json` into `_ => {}` (`index.rs:321-328`) — **zero triples**;
- Rule 22(b)'s filter `dt != RESOURCE && dt != RESOURCE_ARRAY` **passes**, then `iris_of` calls
  `as_iri()`, which returns `None` for `Value::Json` (`resource.rs:140`) — **zero checks**.

Neither errors. `lexicon:cat` and `lexicon:sem_type` are declared reference-typed on every lexicon
entry and contribute nothing to either mechanism. Nothing is *unchecked* — Rule 21 covers those slots
— but not by the mechanism the declaration names, which is the same "looks like it worked" shape as
the `declaration_order` walker.

`core:ctor_type` is the outlier that forced that walker: `core:json`, no `class_types`, so no rule
validates it at all.

**Decision: normalise all 25 onto `core:inductive`**, each keeping its own `class_types` target.
`core:ctor_type` additionally gains `class_types eigentt:TypeExpr`, which it has never had. This is a
bootstrap-ontology edit and therefore requires a reseed.

**The `stats` block is not incidental.** Seven of the twenty-four are the statistics institution's own
inputs — the sample set, the effect size, the variance assumption. Those are the values a verdict is
computed from, and they are exactly what
[D80](d80-witness-and-institution-machinery.md) §3 needs to enumerate. They are unindexed today for
this reason and no other.

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

#### 2.2.1 What a mention names: the inductive, not the constructor

An inductive type has a chain-resolvable IRI. **Its constructors do not.** `InductiveCtorDecl { name,
typ }` lives inside the inductive's declaration and carries no IRI (`nbe/term.rs:507`); constructor
identity is `(inductive IRI, constructor name)`, which is what the D47 wire has always carried —
`CtorApp(D, c)` plus an `App` spine (`term.rs:220-231`).

**This is the correct shape, not an omission.** Constructors are *closed*: a type's constructors are
exhaustively given by its declaration, which is what makes case analysis and the recursor sound. That
closedness is exactly what distinguishes them from resources, which are open-world — anyone may add an
instance of a class in a later layer, and nobody may add a constructor to an inductive in a later
layer. Giving a constructor a chain IRI would state the wrong thing about it. The only information
such an IRI could carry is a back-reference to the parent inductive, and the data structure does not
need one: the parent is the enclosing declaration.

**The representation currently says otherwise, by accident.** The ESL compiler builds each
`core:InductiveCtor` as `Resource::new({parent_iri}:{ctor_name})` (`esl/compile.rs:2189-2197`) rather
than `Resource::new_embedded()`, so every constructor payload carries an `@id` that looks
chain-resolvable and is not. It is stored as `Value::Embedded` inside `core:ctors`
(`compile.rs:2247-2250`), so nothing resolves it, and `extract_indexable_triples`' `RESOURCE_ARRAY`
arm skips `Value::Embedded` anyway.

**That `@id` is written and never read.** Every consumer goes through the `core:ctor_name` *property*
— `decode_ctors` (`program/ground.rs`), the ESL printer, the institution dispatch paths. Even the one
place that uses the `{parent}:{ctor_name}` string form, the compiler's `ctors_by_short_name`,
**reconstructs** it from the parent's `resource.id()` plus the embedded ctor's `ctor_name`
(`compile.rs:299-316`) rather than reading the `@id` that is sitting right there. P4 removes it.

**So a term mentioning `cat_np` emits a mention of `lexicon:Cat`.** The projection is coarser than
per-constructor, and that costs nothing here: every constructor of a sealed inductive is sealed with
it, so those mentions are dropped either way (§2.3). Where it *would* matter is a term mentioning an
unsealed inductive — there the mention is to the type, and a dependent is enumerated whenever any
constructor of it is rebound. That is sound (over-approximate, never missing), and inductives are
sealed, so the over-approximation is unreachable in practice.

### 2.3 Sealing the inductive vocabulary

Indexing every mention naively is expensive at lexicon scale, and expensive in a specific,
diagnosable way. A lexicon entry's `cat` is `cat_np(wn:n00001740, num_sg)`, whose head and second
argument both mention `lexicon:Cat` and `lexicon:Num` (§2.2.1); multiplied over a 7.6M-entry chain
those posting lists have millions of members. Not *wrong* — rebinding `lexicon:Cat` genuinely would
invalidate every entry — but nothing should be able to rebind it.

**The rule: an `InductiveType` may not be redefined.** Constructors need no separate clause: they have
no chain-resolvable identity of their own (§2.2.1), so redefining the inductive is the only way to
change them, and it changes the whole constructor set at once. D76 made the reason literal — the chain
binds names to declarations and `Env::lookup` returns *one* `InductiveDecl`. For a class, "add a
parent" is a monotone edit the alignment layers depend on. For an inductive there is no monotone edit:
changing constructors changes the type, and every committed term mentioning it silently means
something else.

Two static scans over the repo's ontologies, experiments and demo files:

| candidate rule | scope | files that violate it today |
|---|---|---|
| seal **all** bootstrap IRIs (~1211) | 21 compiled-in ontologies | **6** |
| seal **`InductiveType` declarations** (42 ESL `data` + the JSON side) | the term vocabulary | **0** |

The blanket rule is too broad, and the six say why. Five are parsing probes redefining closed-class
*entries* (`resource lexicon:among_prep : lexicon:LexicalEntry`) — instances. The sixth is
`ontologies/encoding/claim-kind-alignment.esl`, which is chain-loaded in the demo and redeclares
`enc:Finding` / `Observation` / `Classification` with lexicon parents added; its header names the
idiom as *"the layered-resolution pattern the wordnet↔umls alignment established."* Sealing all of
bootstrap breaks that. Sealing inductive declarations costs nothing — zero violations across the whole
tree — which is the right time to impose a rule.

**What the seal buys the index.** Mentions whose object is sealed are dropped: they can never enter a
`rebound` set, so their posting lists can never be queried. In `cat_np(wn:n00001740, num_sg)` that
removes both inductive mentions — `lexicon:Cat` and `lexicon:Num` — and they are the ones with
millions of members. `wn:n00001740` survives, and must: the alignment layer redefines WordNet sense
classes, so that edge is one a rebinding can actually break.

Prediction, to be confirmed by P3's measurement rather than assumed: a lexicon entry's retained
mentions come from the *unsealed* IRIs in `cat` and `sem_type`, which is the entry's sense class and
little else — order one per entry against the ~7.6M `lexicon:sem` triples the index already holds.

**What the seal does not do.** It does not address the witness defect
(`layer/witness_index.rs:1184`), where credit survives redefinition of a class a proposition
quantifies over. The rebound name there is `Dog`, a **class**, and classes stay redefinable by design
(§5). That is [D80](d80-witness-and-institution-machinery.md) §2, and it is a separate defect on a
separate trigger — related to the seal only in sharing `conjunction_entails` as its direction test.

---

## 3. Phases

Each phase begins with a code audit, per the discipline D76's phases established — every one of its
seven audits corrected something the design had asserted.

- **P1 — the seal** (§2.3). A validation rule refusing a layer that redefines an `InductiveType` — which seals
  its constructor set with it. Measured at zero violations, so it lands before anything depends on it.
  Gate: the rule fires on a hand-built violating layer; the full workspace suite, the demo and the
  parse gate are unperturbed. **No reseed** — it adds a rule, not an ontology edit.
- **P2 — the declarations** (§2.1). All 24 `core:resource` / `core:resource_array` properties ranged
  at an `InductiveType` become `core:inductive`, each keeping its own `class_types` target;
  `core:ctor_type` becomes `core:inductive` + `class_types eigentt:TypeExpr`. **Bootstrap edit, so a
  reseed**; batch with any other pending bootstrap change. Gates: reseed completes at the current
  resource count with 0 errors; `ctor_type` now reaches Rules 16/21, asserted by a malformed
  `ctor_type` that previously loaded and now does not; and the scan that produced §2.1's inventory
  re-runs clean — **zero properties ranged at an `InductiveType` still declared `core:resource`**, so
  the next one added is caught rather than sampled for.
- **P3 — the indexer arm** (§2.2). `wk::INDUCTIVE` in `extract_indexable_triples`, emitting deduped
  `core:mentions` triples and skipping sealed objects. Gate: index growth on the lexicon chain is
  **measured, not estimated**, against the ~1-2-per-entry prediction; `declaration_order`'s bespoke
  walker is deleted and its `MutualInductives` tests still pass on the shared extraction.

- **P4 — drop the vestigial constructor `@id`** (§2.2.1). `Resource::new(ctor_iri)` →
  `Resource::new_embedded()` in the ESL compiler's ctor construction. Chain-format change, so it
  **batches with P2's reseed** and costs nothing extra. Gates: the full workspace suite and the ESL
  round-trip (print → reparse → identical term) are unperturbed — `esl/print.rs` reads `core:ctor_name`
  and must not regress to the `@id`; `ctors_by_short_name` still resolves qualified constructor
  references, since it reconstructs the string form from the parent's `id()` and `ctor_name`; and a
  reseed produces ctor payloads with no `@id`.

- **P5 — level-parameterise the recursor motive** ([#228](https://github.com/eigenius/eigenius/issues/228)).
  The motive's codomain is the constant `Sort(2)`, so no recursor in the system can eliminate into
  `Type 1` or above. #228 gates this on "#188's residual — declaration-level uparams and
  `Const(iri, levels)`", and **that gate lifted when D76 Phase E2 landed both** (`term.rs:433`,
  `term.rs:64`); #228's other half, the comment that claimed a ceiling the code did not have, was
  already fixed by D76 Phase F. The remaining change is D75 §8 Q4's option 4c: `I.rec.{u}` with motive
  `I(params) → Sort u`, so the two-way choice between `sort(0)` and `sort(2)` becomes *`u` pinned to 0*
  vs *`u` free*. `large_elim_admitted` keeps its exact meaning and call site. Chain-format change
  (`InductiveRec` gains a level), so it **batches with P2's reseed**. Gate:
  `large_elimination_is_capped_at_set_not_type_n` (`nbe/level.rs`) **flips** and is renamed; the
  Prop-without-singleton-elim case is still refused.
- **P6 — qualified constructor syntax** ([#24](https://github.com/eigenius/eigenius/issues/24)).
  `ex:Nat:succ` in expression position, so two inductives in one file declaring the same constructor
  name can be disambiguated instead of rejected — which is what `resolve_ctor_iri` tells the user to
  work around by renaming (`esl/compile.rs`). Parser-only, ~50 lines by the issue's estimate. **The
  issue's rationale needs rewriting first**: it argues from "each constructor has a canonical IRI…
  lookup is IRI-keyed", which §2.2.1 shows is false and P4 removes. The feature is *better* motivated
  under the closed reading — `Nat:succ` names the pair `(inductive, ctor name)`, which is the actual
  identity. No reseed. Gate: a file with two inductives sharing a constructor name compiles, and the
  ESL round-trip preserves the qualified form.

**Order is not arbitrary.** The seal first, so the indexer can rely on it; the declarations next, so
the indexer has a `core:inductive` predicate to match; the arm last. P4 and P5 ride P2's reseed and are
otherwise independent — either can land any time after P2 is written and before the reseed runs. P6
touches neither the chain nor the index and can land whenever. **P2 is the only reseed across all six**,
which is the reason P4 and P5 are here rather than filed for later: each is a chain-format change that
would otherwise need a reseed of its own.

---

## 4. What this unblocks

- **[D80](d80-witness-and-institution-machinery.md)** — binding-aware witness lookup needs the name
  set of an attested proposition, which becomes one `core:mentions` range query.
- **[D77](d77-merge-as-a-pushout-of-environments.md)** — merge's term-dependent enumeration is the
  same query. Without §2.2 it would be a third hand-rolled full-chain walker.
- **The linear commit path.** Rule 22 currently checks nothing for term-valued slots declared
  `core:resource` (§2.1); after P2 they are `core:inductive` and Rule 21 owns them unambiguously.

## 5. What this does not cover

- **Classes and properties stay redefinable.** §2.3 seals inductive-type declarations only.
  Redefining a class to add parents is a load-bearing modeling idiom — the wordnet-umls alignment and
  `claim-kind-alignment.esl` both rely on it. Sealing more would trade a checkable hazard for a
  blocked workflow, and checking that hazard is D77's subject.
- **Genuinely opaque JSON stays `core:json`.** The Julia solver payloads (`primal_solution_kv`,
  `witness_data`, `trajectory_u`) are institution-interpreted blobs with no typed-reference
  semantics. §2.1 moves only the properties that carry D47-encoded terms.
- **Minting constructor IRIs — declined on principle, not on cost.** `nbe/term.rs:227-230` records
  that nanoda gives each constructor its own `Const` while here they are not chain-resident, and that
  minting IRIs "is a chain-format change and belongs with E2". **D76 Phase E2 shipped without it** (it
  carried universe levels), so that pointer is stale. It stays declined, and §2.2.1 gives the reason:
  constructors are closed and resources are open, and a chain IRI asserts openness. P4 moves the
  representation *further* in this direction by dropping the vestigial `@id`, not closer to minting
  real ones.
- **Proposition identity.** Unchanged and environment-blind; that is D80's subject.

## 6. References

- D76 (the typing environment; Phase A is where the second walker was written)
- D32 §3.5 (`core:inductive` and singleton `class_types`), D47 §5 (`ConstRef` resolution)
- D77 §3 (the consumer that found defect #1), D80 (the consumer that finds it on the linear path)
- D75 §8 Q4 (option 4c — the recursor motive as a level parameter), #228, #24, #188 (E2's residual)
- `layer/index.rs` (`extract_indexable_triples`), `layer/declaration_order.rs`,
  `validation/rules/inductive.rs`, `validation/rules/eigentt_value.rs`,
  `validation/rules/reference_integrity.rs`
