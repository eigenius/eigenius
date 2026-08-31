# P0 — measurements before building

**Status: measurement.** Run `2026-08-29` against the working tree at `5a3a1f7`
(branch `d81-epistemic-stack-analysis`, clean). Executes P0 of
[`judgements-warrants-build-plan.md`](judgements-warrants-build-plan.md). No source change.

**Read the method before quoting a number.** §0 of the build plan recorded three counts that
were wrong in its first draft, and the cause was that nobody wrote down the scope. Every count
below names the directories it was taken over and the pattern it matched.

## Method

Four scopes, used throughout. All exclude `target/` and `node_modules/`.

| scope | directories |
|---|---|
| `code` | `kernel/ crates/ cli/` — **including** `tests/` |
| `ont` | `ontologies/` |
| `art` | `demo/ experiments/ notebooks/` |
| `docs` | `docs/` |

Occurrence counts are `grep -roE <pattern> <scope> \| wc -l`; file counts are `grep -rlE`.
**`grep -c` is not used** — it counts matching *lines*, and the generated ontologies are
single-line JSON, so a line count understates them by three orders of magnitude.
Rust-only counts additionally restrict to `kernel/src` + `crates/*/src` and drop `*/tests/*`,
which is the build plan's stated method.

---

## P0.1 — the lexicon survives check mode

**This was the plan's largest unknown. It resolves in the cheap direction: P2 is a rule change,
not a data migration.** The risk row for P2 ("if the failure rate is material, P2 splits into a
repair pass and a rule change") does not fire.

Measured over the converted lexicon chain (`wordnet-chain/`, `umls-chain/`) plus
`ontologies/lexicon/`. These ESL files are exactly what a load commits, so the population is the
committed population.

| slot | values | shape |
|---|---|---|
| `lexicon:cat` | **2,641,933** | 8 distinct outermost constructors, all of `lexicon:Cat` |
| `lexicon:sem_type` | **2,641,933** | all types (`Set`, class IRIs, arrow types, `Prop`) |
| `lexicon:term` | **44,509** | **0 bare lambdas**; 44,443 annotated `( … : T )`, 2 ctor/ref |
| `lexicon:prop` | 0 | no committed values |

`lexicon:cat`'s eight outermost constructors and their counts:

```
2062659 cat_n      299302 cat_np     149509 fwd        81140 bwd
  49253 cat_measure    35 cat_forall     34 cat_fin_forall   1 cat_pp
```

Every one is a constructor of `data lexicon:Cat : Type 1`
(`lexicon-ontology.esl:250`), applied saturated. Inference on a saturated constructor
application yields the family, so `check(v, lexicon:Cat)` succeeds by construction for all
2.64M values. Rule 21 already runs `check_infer` over them and discards the result; P2 asserts
the result it already computes.

`lexicon:term` is the slot that could have failed, and does not. All 44,509 values are
`Ann(e, T)`: 44,443 on one line (44,440 of them in `wordnet-chain/wordnet-00{2,3}.esl`), 64
across multiple lines in `closed-class.esl`, 2 constructor applications. **Not one bare lambda.** The ontology already
instructs this — `lexicon:term`'s description reads *"For an unsynthesizable λ-term, author it
annotated: `(fun … => … : T)`"* — and the instruction was followed, because inference is what
Rule 21 runs today and a bare lambda would not have committed. `check_infer(Ann(e,T))` **is**
check mode, so for this slot P2's rule is already executing; only the assertion is missing.

**Confirmed against the current importer, not just the 2026-08-03 files.** `dcg/glossary.rs`
(2026-08-26), `dcg/augment.rs` (2026-08-20) and `crates/eigenius-umls/src` (2026-08-20) all
changed after the tracked chain was converted, so `wordnet-import` was re-run at HEAD into a
scratch directory. It reproduces the shapes exactly: 471,477 `cat` (WordNet's share), 44,440
`term`, **0 bare lambdas**.

**The two runtime producers cannot introduce a new shape.** `glossary.rs:243` and
`augment.rs:380-381` *read* an existing entry's `cat`/`sem_type` and clone it
(`augment.rs:407,412`); `glossary.rs` derives `sem_type` through `denote_cat`, the ⟦·⟧
homomorphism, which yields types. Neither writes `lexicon:term`. The shape space is therefore
closed by the importers plus `closed-class.esl`, which is what was measured.

**Caveat, stated rather than buried.** This measures decoded *syntax* against the intended type,
not a `check` call per value. The inference-to-check argument above is what carries it: for
`Ann` slots check mode is already running, and for `cat` the family is forced by the constructor.
A per-value `check` run needs a store at HEAD, which needs the reseed P0.6 did not get.

---

## P0.2 — the grade classes have no structural readers

**Confirmed mechanically. P5 is a deletion, not a consumer migration.** D81 §5.1 is re-derived,
not inherited.

Test-module membership was determined by brace-counting each `#[cfg(test)]` block rather than by
assuming test code sits at the end of the file.

| | count |
|---|---|
| Rust files matching `{Declared,Observed,Derived,Verified}Resource` | **26** |
| …plus files reached only via `wk::*_RESOURCE` constants | **29** |
| total occurrences | 138 |
| inside `#[cfg(test)]` | 70 |
| **non-test occurrences** | **68**, in **21** files |
| — comment / doc | 46 |
| — writer or declaration | 22 |
| — **reader** | **0** |

**The plan's 26 is the class-name-literal count and misses three files.**
`kernel/src/layer/index.rs`, `kernel/src/layer/merge/conflict.rs` and `kernel/src/nbe/check/mod.rs`
name the same IRIs only through `wk::DECLARED_RESOURCE`. All three occurrences are inside test
modules, so the conclusion is unchanged — but a sweep that greps the class names alone will miss
them during P5, and they are in the kernel.

**Eight of the 29 files carry occurrences only in test modules**: `eigenius-obograph/src/lib.rs`,
`commit/orchestrator.rs`, `layer/index.rs`, `layer/merge/conflict.rs`, `layer/witness_index.rs`,
`nbe/check/mod.rs`, `validation/mod.rs`, `validation/rules/eigentt_value.rs`.

**The one read-shaped site is not a reader.** `institution/dispatch.rs:540` is
`if !has_class(&classes, wk::DERIVED_RESOURCE)` — an idempotency guard preventing a double
append, inside a writer. No site grants an entitlement on grade-class membership.

---

## P0.3 — names the design reuses with a different meaning

The plan knew about `Warrant`/`Grade`. **Four more, none of them in the plan.** Three are in
`objective-ontology.esl` and `lexicon-ontology.esl`, which P5 already has to open.

### 1. `lexicon:grade` is a third consumer of `reflection:EpistemicStatus`

P5 states the class *"has two consumers in a different ontology"* — `objective:acceptance_grade`
(`:163`) and `objective:axiom_kind` (`:194`). There is a third:

```
property lexicon:grade : core:resource {          # lexicon-ontology.esl:422
    class_types reflection:EpistemicStatus;
    allows_only epistemic:observed, epistemic:declared,
                epistemic:derived, epistemic:verified;
    domain lexicon:LexicalEntry;
}
```

Its description — *"The §8 loop climbs Observed → Declared → Derived → Verified"* — is the
four-grade lattice the design rejects, stated as a progression. `allows_only` is enforced at
commit, so deleting the four `epistemic:*` individuals leaves this property typed at an empty
enumeration exactly as it leaves the objective pair. **Its domain is `lexicon:LexicalEntry`, so
it is the highest-cardinality consumer in the tree by four orders of magnitude** — see P0.5.

### 2. `objective:warrant` is a second holder of the word *warrant*

```
property objective:warrant : core:resource_array {   # objective-ontology.esl:242
    description = "Why a selection holds: the DesirableProperties / CompetencyQuestions /
                   Axioms that justify it.";
```

P1.3 declines the `warrant:` namespace because `reflection:warranted_by` holds the word, and P5
undertakes to settle that property. `objective:warrant` is named nowhere in the plan and is the
same shape — a rationale pointer, not the paper's computed warrant. **Both must be settled
together or the namespace question reopens.**

### 3. `objective:witness` is an untyped pointer at the `Is*As` families

```
property objective:witness : core:string {           # objective-ontology.esl:199
    description = "IRI of the admitted witness backing this Axiom (an IsObservedAs /
                   IsDeclaredAs witness, or a reference:Citation).";
```

A `core:string` naming a witness. P7 moves `witness:Is*As` into kernel base vocabulary; this
property points at them as text, so the move does not reach it. It is `reflection:source`'s
defect — a relation stored as a string — one ontology over, and P5's retype argument applies to
it verbatim. `objective:WitnessKind` (`:106`) is a parallel operational enum alongside it.

### 4. `lexicon:Sentence` and `lexicon:term` already hold the words P1.3 assigns

`lexicon:Sentence` (`:506`) is a composed natural-language sentence; P1.3 renames
`reasoning:ReasoningSentence` to `justification:Sentence`. Different namespaces, so no IRI
collision — but the DCG pipeline's `SentenceEncoding`, `SentenceOutcome`, `SentenceResolution`
and `SentenceSelection` are all the *lexicon* sense, and after the rename the bare word denotes
two unrelated things one namespace apart.

Likewise `lexicon:term` (`:499`) is `core:inductive` with `class_types eigentt:TypeExpr`. After
P1 renames `eigentt:TypeExpr → eigentt:Term` and P1.3 renames `reasoning:justification →
justification:term`, the tree carries three `term`s: `eigentt:Term` (a term of the theory),
`lexicon:term` (a lexical entry's λ-semantics) and `justification:term` (the justification slot).

**No collision found for `Judgement`, `Certificate`, or `Projection`.** `urn:eigenius:logic`
exists as a namespace, so `eigentt:Logic` is a same-word-different-namespace case, not a clash.

---

## P0.4 — `Sum` and `SpecStr`

**`Sum`: the plan is right.** The only occurrences in `ont`/`art` are the declaration
(`reasoning.esl:79`) and the two rules that mention it (`:148`, `:156`). No authored artifact
constructs a `Sum`. P4's strengthening of `sum_l`/`sum_r` breaks nothing, which is the argument
for doing it before the first real `Sum` commits.

**`SpecStr`: the plan is wrong.** P0.4 says *"its `SpecStr` uses are three fixtures"*. There are
**5 authored artifacts** carrying constructor applications, plus 3 fixtures:

| file | sites | kind |
|---|---|---|
| `experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl` | 2 | **authored — the WRN chain** |
| `experiments/benchmark/tasks/sab/16-compound-filter/tracer-chain.esl` | 2 | authored |
| `experiments/benchmark/tasks/sab/18-dili-rf/tracer-chain.esl` | 2 | authored |
| `demo/prose-to-formulas-v2/inference.esl` | 2 | authored |
| `notebooks/examples/stats-and-reasoning.json` | many | authored (cell sources) |
| `crates/eigenius-statistics/tests/fixtures/d39_composition.esl` | 6 | fixture |
| `kernel/tests/fixtures/universal_rule.esl` | 2 | fixture |
| `kernel/tests/fixtures/spec_poly_set_domain.esl` | 2 | fixture |

**P4 removes the `SpecStr` term constructor while keeping the `spec_poly` rule**, so each of
these needs its justification term rewritten (drop the wrapper) and its certificate's result
index changed. **One of them is `05-phase1-discovery.esl` — part of the WRN demo the plan uses
as its own end-to-end regression gate.** P4 is correspondingly larger than "three fixtures", and
its edit lands on the artifact every phase is checked against.

---

## P0.5 — inventory

**There is no persisted chain at HEAD to inventory.** The most recent snapshot is
`../db-snapshot/wordnet-umls-aligned-2026-08-03-specpoly`; `ontologies/` and
`kernel/src/bootstrap/` have taken **10 commits** since, including
`51d284c` (*universes as levels*) and `ab83efa` (*P2 residue*). A chain is rooted at the
bootstrap it was seeded with, so those snapshots fail closed on `ManifestDrift`. The inventory
below is therefore over what a load *would* commit: the converted chain plus the authored ESL.

**The lexicon chain carries none of the justification vocabulary.** `DerivedEvidence`,
`IsDerivedAs`, `DeclaredResource` and `DerivedResource` are all **0** across `wordnet-chain/`
and `umls-chain/`.

**It carries the epistemic individuals, 2.64M times.**

| | count |
|---|---|
| `lexicon:grade` values in the converted chain | **2,641,713** |
| — all of them `epistemic:declared` | 2,641,713 |
| — any other `epistemic:*` value | 0 |
| `epistemic:*` in `ont` + `art` | 26,725 |

**P5 deletes `reflection:epistemic:{declared,observed,derived,verified}`.** Through
`lexicon:grade`'s `allows_only`, that invalidates every lexical entry on the chain. The plan
sizes P5 as *"the 21 Rust writers and 9 ontologies"*; the actual reseed-side surface is 2.64M
resource stamps, reachable only via the property P0.3 found. It is not a data migration — the
chain is rebuilt from the importers, so the fix is one line in `wordnet-import`/`umls-import`
plus one in `lexicon-ontology.esl` — but it **is** a bootstrap edit that must land in the same
reseed as the rest of P5, and it is invisible from the plan's inventory.

The 26,725 authored occurrences concentrate in parse probes, not in ontologies:
`frame2223-whole-lexicon.esl` 18,294, `alignment.esl` 7,260, `vpadj-crossproduct.esl` 539,
`closed-class.esl` 220.

**`DerivedEvidence` / `IsDerivedAs` in authored artifacts: 153 occurrences in 25 files**,
reproducing §0's corrected count. The concentration is the WRN chain —
`04-phase1-recompute-conclusions.esl` alone holds 60, and 8 of the 25 files are under
`experiments/publications/wrn-helicase/`.

### Corrections to §0

| §0 | measured | note |
|---|---|---|
| grade classes, 26 Rust files | 26 by class name, **29** including `wk::` constants | the 3 extra are kernel, test-only |
| `*Evidence`, 480 occ / 56 files | `code` 154/25, `ont` 30/3, `art` 282/19 → **466 / 47**; +`docs` 263/27 → 733/75 | §0's scope is unrecoverable; use the split |
| `reasoning:` short-form, 521 | `code` 193, `ont` 130, `art` 299 = **622**; +`docs` 218 = 840; **843** repo-wide | the last 3 are outside all four scopes |
| `TypeExpr`, 52 ontology sites / 21 Rust files | **52 / 21** | reproduces |
| schema-org carries 2114 grade-class occurrences | **2114** | reproduces (needs `grep -o`, not `-c`) |
| authored grade-class surface | **1581 occ / 59 files** — but 1355 are in 9 `run.log` files | real authored surface **226 in 50 files** |

---

## P0.6 — baseline: **not run**

The parse gate and the WRN demo were not run, and no baseline number is recorded. Stating that
plainly rather than recording a number taken under different conditions:

- **It needs a reseed at HEAD.** Bootstrap has drifted 10 commits past every snapshot on disk
  (above), so there is no store the gate can open.
- **The recorded baseline is not reproducible without live LLM calls.**
  `experiments/parsing/baseline.json` pins `profile: release`, `features: [use-llm]`,
  `reranker: AnthropicSenseRanker (live), recorded`, snapshot
  `wordnet-umls-aligned-2026-08-02-consolidated`, commit `c1adb65` (2026-08-02).
  `ANTHROPIC_API_KEY` is set, so this is possible, but it spends and it is a *draw* — the
  baseline itself records being the "drift-free REPLAY of draw 1 of 3".
- **The reseed drops the docker volume** (`docker volume rm eigenius_eigenius_db`) and must be
  invoked with an explicit UMLS scope. `--umls-all` is the only scope comparable to
  `baseline.json`; the subset silently drops whole concepts.

**Everything above is independent of P0.6** — P0.1–P0.5 are measurements over the tree and the
converted chain, and none of them needs a store. What P0.6 gates is the regression comparison
for P1 onward, not the sizing decisions P0 exists to make.

---

## What this changes in the plan

1. **P2 is a rule change.** P0.1's failure rate is zero. Drop the P2 risk row.
2. **P5 is a deletion.** P0.2 confirms no reader — but sweep on `wk::*_RESOURCE` as well as on
   the class names, or three kernel files are missed.
3. **P5 gains `lexicon:grade`** (P0.3 §1, P0.5): a third `EpistemicStatus` consumer, 2.64M
   stamps, a bootstrap edit, absent from the removal inventory and the retype table.
4. **P5 gains `objective:warrant` and `objective:witness`** (P0.3 §2, §3). The warrant-naming
   question is not settled by resolving `reflection:warranted_by` alone.
5. **P4 is larger than stated** (P0.4): 5 authored artifacts carry `SpecStr`, including the WRN
   chain that is the plan's own regression gate — not 3 fixtures.
6. **§0's counts need their scope.** Three of six do not reproduce without one; the table above
   supplies the split.
