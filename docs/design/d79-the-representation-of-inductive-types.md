# D79 — The representation of inductive types

**Status: IMPLEMENTED**, merged `2026-08-26` in [#229](https://github.com/eigenius/eigenius/pull/229).
All seven phases landed; §7 is the build log, and records that three of the seven did not do what
this plan said they would. Completes the inductive-type work D76 began, and is a prerequisite for
[D77](d77-merge-as-a-pushout-of-environments.md) and [D80](d80-witness-and-institution-machinery.md),
both of which remain design-only. Split out of D77 on `2026-08-25`, which had accumulated three
separable projects.

D76 made the layer chain the typing environment: the chain binds **names to declarations**, and
`Env::lookup` returns one `InductiveDecl`. This document finishes the representation half of that
change. **Seven defects**, each found by a consumer that needed something the representation does
not provide — the first three were the split's motivation, and #4-#7 were found while building
the fix for them:

| # | defect | found by |
|---|---|---|
| 1 | three declarations carry D47-encoded terms, and one of them silently no-ops | D77 §3 needing to enumerate term dependents |
| 2 | an encoded term contributes no triples, so nothing can query what it mentions | D76 Phase A, forced to hand-write a second reference walker |
| 3 | an inductive constructor is redefinable, so every term mentioning it can silently change meaning | the cost analysis for fixing #2 |
| 4 | every constructor payload carries a vestigial `@id` that reads as chain-resident openness | asking what a constructor's identifier is for |
| 5 | the recursor's motive codomain is a hard-coded `Sort(2)`, capping large elimination at `Set` ([#228](https://github.com/eigenius/eigenius/issues/228)) | its stated gate lifting when D76 Phase E2 landed |
| 6 | qualified constructor references do not parse, so same-named ctors cannot be disambiguated ([#24](https://github.com/eigenius/eigenius/issues/24)) | reading `resolve_ctor_iri`'s own error message |
| 7 | `core:List` is kernel-intrinsic but appears in authored ESL and persisted terms, and two decoders already disagree about what it means | asking whether it crosses process boundaries |

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
**type-checks** every `eigentt:Term`-ranged value, rejecting unresolved `ConstRef`s
(`validation/rules/inductive.rs:171`, `validation/rules/eigentt_value.rs`).

**The misdeclaration is systemic, not a few strays.** A full scan of every property whose
`class_types` names one of the 43 `InductiveType` declarations:

| declaration | count | validated | indexed |
|---|---|---|---|
| `core:inductive` | **5** | Rules 16 + 21 | no — and says so |
| `core:resource` | **22** | Rule 21 | **claims to be, isn't** |
| `core:json` (no `class_types`) | 1 — `core:ctor_type` | **nothing** | no |

Every misdeclaration is `core:resource`; **none is `core:resource_array`** (§2.1.1).

Correct today: `eigentt:axiom_statement`, `definition_type`, `definition_body`,
`formulas:operator_signature`, `lean:proposition`. Everything else reaches for `core:resource` —
`core:resource` is the default and `core:inductive` is the exception, which is why an earlier draft of
this section, working from a sample, reported four misdeclared properties rather than twenty-two:

| namespace | misdeclared properties |
|---|---|
| `lexicon` | `cat`, `sem_type`, `term`, `prop` |
| `reasoning` | `proposition`, `candidate_proposition`, `certificate`, `justification` |
| `stats` | `sample_set_value`, `effect_size`, `variance_assumption`, `directionality`, `outlier_exclusion`, `multiple_comparison_correction`, `autocorrelation_structure` |
| `objective` | `proposition`, `option_claim` |
| `core` | `type_name`, `param_kind`, `result_sort` |
| `reflection` / `enc` | `canonical_proposition`, `antecedent_term` |

Note the range targets are not only `eigentt:Term`: `core:result_sort` ranges at `core:Level`,
`reasoning:justification` at `reasoning:JustificationTerm`, and the seven `stats` properties at their
own institution inductives. The defect is about the *declared data type*, not about which inductive.

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

**Decision: normalise all 23 onto `core:inductive`**, each keeping its own `class_types` target.
`core:ctor_type` additionally gains `class_types eigentt:Term`, which it has never had. This is a
bootstrap-ontology edit and therefore requires a reseed.

**The `stats` block is not incidental.** Seven of the twenty-four are the statistics institution's own
inputs — the sample set, the effect size, the variance assumption. Those are the values a verdict is
computed from, and they are exactly what
[D80](d80-witness-and-institution-machinery.md) §3 needs to enumerate. They are unindexed today for
this reason and no other.

#### 2.1.1 There is no `core:inductive_array`, and there should not be

The question the inventory raises: `core:resource` has an array partner, so does `core:inductive` need
one? **No.**

First, nothing asks for it. The one apparent case —
`objective:options : core:resource_array + class_types objective:Option` — is not one: `objective:Option`
is a `class`, so that declaration is already correct. It appeared in a draft of the table because the
scan matched ESL qualified names by local-name suffix and confused it with the kernel's `core:Option`.
Every real misdeclaration is scalar.

Second, and the reason it would be wrong even with a case: **a list of terms is itself a term.**
`core:resource_array` and `core:value_array` exist because a reference and a scalar have no
representation *inside* the term language — the ontology layer has to express multiplicity for them.
A D47-encoded value does not have that problem: `List A` is an inductive like any other, so "several
of these" is already sayable in the thing being stored. Adding `core:inductive_array` would put a type
constructor at the ontology's data-type layer, duplicating one the type theory already owns, and would
produce a shape the kernel type-checks as an array of terms rather than as the single term it is.

**The real gap is `core:List` itself, and it is not hypothetical — see §2.1.2.**

#### 2.1.2 `core:List` is kernel-intrinsic and already crosses process boundaries

`core:List` is built by `nbe::term::list_decl()` and answered by `Env::intrinsic` in every
environment. It is **not a chain resource**. Three consequences, all of which the codebase already
treats as defects for the sibling case `core:Option`:

**It is in authored content and in persisted terms.** `experiments/lexicon/lexicon.esl:68` declares
`axiom lexicon:forms_complex : core:List(lexicon:Entity) -> Prop`, and `:347` puts
`type_expr( core:List(lexicon:Entity) -> Prop )` in a `lexicon:sem_type` slot — one of §2.1's
twenty-two. Those compile to D47 terms carrying `ConstRef("urn:eigenius:core:List")`, which is
committed and re-read. The name is on the wire whether or not it resolves.

**Two in-process decoders already disagree about it.**

| decoder | `core:List` decodes to |
|---|---|
| `eigentt_type_mirror`'s `ConstRef` arm (`:947`) | `Exp::Const(core:List, [])` — **the inductive** |
| `ground::decode_arg_type` (`program/ground.rs`) | `Exp::EigonClass(core:List)` — **a class marker** |

The second has no `List` arm, so it falls past the five primitives to `names_an_inductive`, which is a
chain lookup and fails, and lands on the `EigonClass` fallthrough. Same IRI, two meanings, no error.
The comment at the *first* site states the principle the second violates:

> *"The canonical built-in `List` is not a chain resource, so it would not resolve below.
> `Env::intrinsic` is the environment's matching answer — the two must agree, or a name means one
> thing to the decoder and another to the type checker (D76 Phase B, sixth correction)."*

D76 Phase B fixed the environment-versus-decoder divergence and did not look for the
decoder-versus-decoder one.

**Out of process, nobody handles it.** A grep for `core:List` across `.rs` / `.ts` / `.jl` / `.R` /
`.lean` outside `kernel/src` finds no handler — only uses. One of those uses is a modelling
distortion already paid for: the Julia DiffEq ontology introduces a wrapper class rather than a
`List<FormulaTerm>` property, explaining that it did so *"because the chain doesn't have a parametric
`core:List<T>` surface committed yet"*
(`julia/institutions/diffeq/declarations/diffeq-ontology.eigon.json:87`).

**The fix is the one already chosen for `core:Option`.** `core:Option` is *also* parametric and *is*
chain-declared (`core-ontology.json:1011`, with `type_params: [A]` and ctors `none` / `some`), so
parametricity is not the obstacle the diffeq comment assumed. `Env::intrinsic`'s own doc says why the
kernel does not keep a private copy of it:

> *"`core:Option` is deliberately **not** here — it *is* a chain resource, and taking the kernel's copy
> would hide any disagreement between the two rather than surface it."*

That reasoning applies unchanged to `List`. P7 declares it, deletes both special cases, and adds the
analogue of `the_chain_and_the_kernel_agree_about_option`.

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
(`layer/witness_admission.rs:1184`), where credit survives redefinition of a class a proposition
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
- **P2 — the declarations** (§2.1). All 22 `core:resource` properties ranged at an `InductiveType`
  become `core:inductive`, each keeping its own `class_types` target;
  `core:ctor_type` becomes `core:inductive` + `class_types eigentt:Term`. **Bootstrap edit, so a
  reseed**; batch with any other pending bootstrap change. Gates: reseed completes at the current
  resource count with 0 errors; `ctor_type` now reaches Rules 16/21, asserted by a malformed
  `ctor_type` that previously loaded and now does not; and the scan that produced §2.1's inventory
  re-runs clean — **zero properties ranged at an `InductiveType` still declared `core:resource`**, so
  the next one added is caught rather than sampled for. The scan must resolve `class_types` by full
  IRI: an earlier version matched ESL qualified names by local-name suffix and reported
  `objective:options -> core:Option`, which is `objective:Option`, a **class** (§2.1.1).
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

- **P7 — declare `core:List` in the core ontology** (§2.1.2). Model it on `core:Option`:
  `is_a core:InductiveType`, `type_params [A]`, ctors `nil` / `cons`. Then **delete both special
  cases** — `Env::intrinsic`'s `LIST` arm (`nbe/env_global.rs:262`) and the decoder's
  (`eigentt_type_mirror.rs:947`) — so the chain is the single answer. Bootstrap edit, so it **rides
  P2's reseed**. Gates: a `the_chain_and_the_kernel_agree_about_list` test mirroring the `Option` one;
  `ground::decode_arg_type` and the `ConstRef` decoder now agree, asserted by decoding the *same*
  `core:List` reference through both and comparing — the assertion that would have caught this;
  `experiments/lexicon/lexicon.esl` still compiles and its `sem_type` still type-checks; and
  `class_types core:List` resolves, which is what §2.1.1 said would be needed if a list-valued term
  slot ever arrived.

**Order is not arbitrary.** The seal first, so the indexer can rely on it; the declarations next, so
the indexer has a `core:inductive` predicate to match; the arm last. P4, P5 and P7 ride P2's reseed and are
otherwise independent — each can land any time after P2 is written and before the reseed runs. P6
touches neither the chain nor the index and can land whenever. **P2 is the only reseed across all
seven**, which is the reason P4, P5 and P7 are here rather than filed for later: each is a
chain-format or bootstrap change that would otherwise need a reseed of its own.

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

---

## 7. Build log

Landed `2026-08-25`--`2026-08-26`, merged in #229. Each phase's audit corrected something the plan asserted, which is the
discipline D76's phases established and the reason they begin with one.

| phase | outcome |
|---|---|
| **P1** seal | as planned. 4 tests |
| **P2** declarations | **22, not 24** — see below. 23 properties edited (22 + `ctor_type`) |
| **P3** indexer arm | as planned; `declaration_order`'s walker deleted in favour of the shared one |
| **P4** ctor `@id` | as planned; also removed a `self.resolve` left unused by it |
| **P5** recursor motive | **already implemented.** Reduced to deleting a stale comment |
| **P6** qualified ctors | **fixed at the lexer, not the parser** — #24's proposal would break annotations |
| **P7** `core:List` | as planned; a round-trip gate caught a real defect in the first draft |

### 7.0 The reseed, and P3's measurement

`scripts/reseed-lexicon-db.sh --umls-all`, `2026-08-25`: **9,439,633 resources across 35 layers, 0
errors** — identical to the pre-D79 baseline. The whole chain revalidates under the new declarations,
the seal, and a chain-declared `core:List`, which is P2's gate.

**P3's index growth, measured rather than estimated** — the number the phase deferred because it
needs a real lexicon chain:

| | store |
|---|---|
| before D79 | 2.741 GB |
| after D79 | 2.869 GB |
| **growth** | **+128.0 MB, +4.7%** — 13.6 bytes per resource |

§2.3 predicted "order one [retained mention] per entry", against the ~7.6M `lexicon:sem` triples the
index already held. At roughly 15 bytes per triple that predicts ~130 MB, so the seal is doing what
the section claimed: without it, the `lexicon:Cat` and `lexicon:Num` posting lists alone would have
multiplied this several-fold.

### 7.1 P2 — the count was 22, and the extra two were a scan bug

§2.1's first inventory reported 24, including one `core:resource_array`
(`objective:options -> core:Option`). That row was wrong: `objective:Option` is a **class**, and the
scan's ESL branch had resolved `class_types` qualified names by *local-name suffix*, matching it
against the kernel's `core:Option`. `objective:selected` fell out for the same reason.

The gate is now `scripts/inductive-ranged-properties.py`, which resolves by full IRI only and is
**both the inventory and the check** — so the two cannot drift, and the specific mistake is named in
P2's gate text.

### 7.2 P5 — #228 was closed by D76 Phase F, not merely unblocked

The plan read #228 as gated on "#188's residual — declaration-level uparams and `Const(iri, levels)`"
and scheduled the fix here once that gate lifted. It had lifted, and **the fix had already landed with
it**: `derive_motive_codomain` (`check/inductive.rs`) applies the motive to fresh generics and infers
what sort the result inhabits, so `Type 1` and above are admitted, and
`check::inductive::tests::a_type_1_valued_motive_is_admitted` runs an actual recursor at each level.
#228's other half — the comment claiming a ceiling the code did not have — was fixed in the same
phase.

What remained was a **stale comment block** still describing the removed constant, stacked directly
above the correct one, ending *"Gated on #188's residual"* — a gate that had since lifted. Two
contradictory explanations of the same code. **#228 is closeable.**

### 7.3 P6 — #24's proposed fix would have broken annotation colons

#24 specifies the change in `parse_qualified_name`: *"greedily consume additional `:Ident` chains"*.
That eats the binder colon — the parser's own comment reserves a standalone `Colon` for `x : T`, and
`ex:Nat : Prop` would have parsed as one name.

The change belongs in the **lexer**, because tightness is already the discriminator:
`tight_qualified_tail` only runs on a `:` with no whitespace before it, and the language has always
required spaces around a binder colon (`x:T` lexes as `QualName("x", "T")`). Continuing the name
across further *tight* `:segment`s gives `ex:Nat:succ` → `QualName("ex", "Nat:succ")` and leaves
`ex:Nat : Prop` untouched, which a test pins.

#24's **rationale** also needed correcting rather than implementing: it argues from *"each constructor
has a canonical IRI … lookup is IRI-keyed"*, which §2.2.1 shows is false and P4 removed. The feature
survives the correction because it never depended on that premise — `(inductive, ctor name)` *is* a
constructor's identity, so the type is the only thing a constructor reference could be qualified by.

### 7.4 P7 — the round-trip gate caught the declaration, not the code

`every_shipped_inductive_round_trips_through_esl` (an existing test the plan did not name) refused the
first `core:List` draft: its nested `type_args` entry carried `arg_name: "elem"`, but a **type
argument is positional and has no name**. Fixed in the declaration rather than pinned as an exception.

### 7.5 Not fixed here, found here

- **A merge rename did not rewrite term references — found here, then fixed here.** `value_mentions_iri` and
  `substitute_iri_in_value` (`layer/merge/resolve.rs`) both stop at `Value::Json`, so a resource whose
  only reference to a renamed IRI was inside a term was neither selected nor rewritten — silent
  corruption, not a stale check. It became fixable the moment P2 landed, because the rule is "descend
  into `Value::Json` only when the property is declared `core:inductive`" and before P2 the declared
  type could not tell a term from an opaque payload. Both halves now take the side's head layer and
  read the carrier's `data_type`. [D77](d77-merge-as-a-pushout-of-environments.md) §3.6 records it.
- **The `db_backed_encoding` snapshot tests race.** Two snapshot-using tests in one binary derive
  their work directory from the *parent* pid, so both copy to the same path and the second fails.
  Pre-existing, environmental, and unrelated to this document; single-threaded runs pass. Not fixed
  because it is a harness bug in a file this work does not otherwise touch.
