# D63 — Phrasal & predicative comparatives (`greater/fewer/more X … than Y`): grounded design

**Status (`2026-07-06`):** Mechanism built + demo-verified (§3). Post-demo the analysis was **refined**
and the deployment plan **corrected** (§4–§5): the governing concept is **not "measure noun"** but a
**cardinality vs degree** split. **#9** (`fewer deletion mutations`) is **cardinality** — any count noun,
no special class, no detection. **#8** (`greater dependence` / `more dependent`) is **degree** — one
scale, anchored on the gradable **adjective**, with the noun as its nominalization. Comparative operators
are closed-class (→ bootstrap); exemplar gradable words are test scaffolding (→ demo lexicon, **not**
bootstrap); general emission is the **importer** (a scoped design effort keyed on gradable adjectives).
Working tree is at `a016cea` (demo baseline); a bootstrap-pollution + reseed attempt (a curated starter
set in `closed-class.esl`) was tried and **reverted** — the placement lesson is in §5.

## 1. Witnessed facts (Derived, `2026-07-06`)

**Nominal route** (`probe_rc2_comparatives`, snapshot `wordnet-umls-all-2026-07-06`):
- Attributive comparatives already parse (positive `S[adj]\NP` reading, morphy `greater`→`great`):
  `a stronger phenotype affects cells` CLOSED×168, `greater dependence affects cells` CLOSED×72.
- The `than`-clause is the gap: `WRN showed greater dependence than genes` GAP,
  `cells contained fewer mutations than genes` GAP — `cat_pp_than` binds only to the *predicative*
  comparative `(S[adj]\NP)/cat_pp_than`, and with `greater` attached attributively the `than`-phrase has
  nothing to bind.
- The two real RC-2 gaps: **#8** `MSI cell lines … greater dependence on WRN than their MSS counterparts`,
  **#9** `… fewer deletion mutations in microsatellite regions than typical lineages`. **#12 is misfiled**
  (its attributive comparative parses; it gaps on `may require` + compound subject).

**Adjectival route** (demo probe, `a016cea`):
- Synthetic predicative comparative works: `HeLa is larger than BRCA1` → `gt(deg_large(hela),
  deg_large(brca1))`.
- Relational gradable adjective works (fixture): `HeLa is dependent on BRCA1` → `gt(deg_dependent(brca1,
  hela), std_dependent)` via `(S[adj]\NP)/cat_pp_arg`.
- **Analytic `more`/`less` is the only gap:** `HeLa is more large than BRCA1`, `HeLa is more dependent on
  BRCA1 than MSH2` GAP. `degree_adverb_items` (lookup.rs) lifts `more`/`most`/`less` only over *adverbs*
  (`more commonly`), transparently — no adjective path, no `more`/`less` lexeme.

## 2. Grounding — expert consultation (`2026-07-06`) + anchors

The five discovery targets and their expert answers (grounded; anchors §6, DOIs `note`-flagged).
**Refined post-demo** where marked.

- **Q1 — a comparative OPERATOR, not the positive adjective.** `greater`/`fewer` compare a scale, not the
  degree of "great"/"few"; the `greater→great` lemmatization is actively harmful (a different proposition).
  **[Refined §4]** the expert's "amount of the extension" (`μ_amount : (Entity→Prop)→float`) conflates two
  mechanisms — #9 is *cardinality* of the extension, #8 is *degree on a scale*; they need different
  treatments.
- **Q2 — stipulate the measure, don't derive it.** An opaque per-dimension entity measure
  `deg : Entity → float` (same shape as `deg_A`) avoids reifying events/sets. The pivotal tractability move.
- **Q3 — the DIRECT / phrasal analysis suffices** (Bhatt & Takahashi): subject-vs-subject contrasts → a
  3-place combinator `λμ.λy.λx. gt(μ(x), μ(y))`; reduced-clausal only for subcomparatives / adjunct
  contrasts (not here).
- **Q4 — CCG attachment:** `than Y` attaches to the VP/S (not the object NP); the object GQ passes the
  measure up and the `than`-phrase consumes it. (Demo realized this as `/cat_pp_than` on the object-GQ
  result — no new feature; §3.)
- **Q5 — an opaque per-dimension `μ`/`deg` is faithful** for a graded KG: transitivity + inverse hold;
  decomposition / underlying-event are not needed for the graded claim.

## 3. The mechanism — built + demo-verified (`a016cea`)

Green on the demo grammar (no reseed):
- **`cat_measure`** in `lexicon:Cat` (⟦·⟧ = `Entity → core:float`, `denote_cat` arm) — the scale-supplying
  category; `*greater gene` is rejected because `gene` isn't `cat_measure`.
- **Degree operator** `greater` = `( ((S\NP)/cat_pp_than) \ ((S\NP)/NP) ) / cat_measure`, sem
  `λμ.λV.λy.λx. gt(μ(x), μ(y))`; `fewer` the LESS variant `gt(μ(y),μ(x))`. `[comp]` = the `/cat_pp_than`
  on the object-GQ result (reuses the predicative comparative; no new feature).
- **Predicative comparative** (pre-existing, D63 §8.12): gradable adjective `deg_A : Entity→float`;
  `larger` = `(S[adj]\NP)/cat_pp_than`, `λy.λx. gt(deg_large(x), deg_large(y))`. Relational adjectives
  add a `/cat_pp_arg` ground (§1 fixture).
- **Verified:** `HeLa affects greater dependence on BRCA1 than MSH2` → `gt(mu_dependence(brca1, hela),
  mu_dependence(brca1, msh2))` (exact, type-checks to `Prop`); `dependent on BRCA1` →
  `gt(deg_dependent(brca1, hela), std_dependent)`. Demo suite + kernel lib green.

## 4. Refined understanding — #8 (degree) vs #9 (cardinality); "measure noun" is the wrong concept

The term **"measure noun" is wrong** — in formal semantics it means a **unit of measure** (liter,
kilogram, degree; the pseudo-partitive "three liters of water", Rothstein 2017). `dependence`/`mutations`
are not units. Worse, it conflates two mechanisms:

**Cardinality (#9).** `fewer/more N` compares a **count**; works on *any* count noun (`fewer
genes/cells/mutations`); μ = |extension|. **No special noun class, no per-noun axiom, no detection.**
`deletion mutations` is a **compound count noun** (N+N, the existing `RefineKind::KindCompound` over
`cat_n`); `in microsatellite regions` a restrictive PP on the noun. So the earlier "#9 = compound
*measure* (a `cat_measure` analogue of `KindCompound`)" diagnosis was wrong — it's a compound **count**
noun; that branch dissolves.

**Degree (#8).** `greater/more/higher N` compares a **degree on a scale**; works only on gradable
elements (`*greater gene`). The scale lives on the gradable **adjective** (`dependent`, `deg_dependent`);
the noun (`dependence`) is its **nominalization**, inheriting the same `deg`. `more dependent on WRN` and
`greater dependence on WRN` **denote identically** — one degree function, two surfaces. The hand-written
`mu_dependence` is `deg_dependent` re-packaged.

**Agreement** confirms the split: the comparative word carries the feature and must agree — `fewer`+count,
`greater`+scalar; `*fewer dependence`, `?greater mutations` are out.

**One operator.** `more` over `deg_A` and `greater` over `μ` are the **same** comparative-degree operator
over a scale `Entity→float`; the synthetic `-er` (`larger`) is it pre-bundled; `fewer`/`more`(count) is
the cardinality variant. One operator family (`more/greater/-er/fewer/less`), fed by either an adjective's
`deg_A` or a count noun's cardinality.

**Detection reframes to the tractable side.** "Which nouns are gradable?" is fuzzy; but gradability is
marked on **adjectives**, and WordNet marks *that* — antonym/gradable adjective clusters + the `attribute`
relation (`heavy/light ↔ weight`). So detect gradable **adjectives**; project their `deg` to
nominalizations via derivational links (`dependent → dependence`); the **relational** ones are adjectives
that subcategorize a PP (`dependent on`, `sensitive to`).

## 5. The design to address #8 and #9

### 5.0 One scale, three frames, one operator family

The unifying object is a single opaque **scale** `deg : Entity → float` (relational: `Entity → Entity →
float`, ground+subject) — the `cat_measure` category (⟦·⟧ = `Entity → float`); relational scales are
`cat_measure / cat_pp_arg`, the ground filled by an `on`/`to` PP. A gradable **adjective** and its
**nominalization** supply the *same* `deg` (`deg_dependent` = `μ_dependence`); a count **noun** supplies a
*cardinality* scale. Every comparative reduces to `gt(deg(x), deg(y))` over that scale; the operators
differ only in (i) count vs degree and (ii) the syntactic frame:

| Surface | Frame | Operator | Scale source |
|---|---|---|---|
| `is more/less dependent on WRN than Y` | predicative `(S[adj]\NP)/cat_pp_than` | `more`/`less` | gradable adjective `deg_A` |
| `shows greater/less dependence on WRN than Y` | object-GQ VP | `greater`/`less` | nominalization `μ = deg_A` |
| `has fewer/more mutations than Y` | object-GQ VP | `fewer`/`more`(count) | count noun cardinality |

`cat_measure` is really the **scale** category (an adjective's `deg` or a noun's `μ`); a rename to
`cat_scale`/`cat_deg` would reflect that. `more` is ambiguous (degree over a scale vs count over `cat_n`)
— two entries. All operators are closed-class.

### 5.1 #9 — cardinality (a grammar rule over `cat_n`; no data, no detection)

`fewer`/`more`(count) select a **count noun directly** and build the cardinality internally:

```
fewer : ( ((S\NP)/cat_pp_than) \ ((S\NP)/NP) ) / cat_n(T, num)
        sem  λN. λV. λy. λx. gt(card(N, y), card(N, x))       -- more(count): gt(card(N,x), card(N,y))
```

- `card : Set → Entity → float` — an **opaque** per-noun cardinality (the verb/containment folded in, like
  the absorbed light verb `V`). A faithful graded claim; defers set reification (§7).
- `deletion mutations` is a compound `cat_n` (existing `RefineKind::KindCompound`); `in microsatellite
  regions` a restrictive PP on the `cat_n` — both refine `N` *before* `fewer` counts it, so they compose
  for free.
- **No importer emission, no detection** — any `cat_n` counts. Selecting `cat_n` directly (not a
  type-changing `cat_n ⇒ cat_measure` lift) keeps the rule from making *every* noun a measure — avoids the
  ambiguity blow-up a free lift would cause.
- Retract the earlier "#9 = compound **measure**" (a `cat_measure` analogue of `KindCompound`): it's a
  compound **count** noun; nothing new is needed there.

### 5.2 #8 — degree (a gradable scale, anchored on the adjective)

The scale lives on the gradable **adjective**; the noun projects the same `deg`. Two operator frames over
one `cat_measure`:

```
-- adjective, predicative (analytic `more`/`less`; synthetic `-er`, e.g. `larger`, is the same, bundled):
more : ((S[adj]\NP)/cat_pp_than) / cat_measure               sem  λμ. λy. λx. gt(μ(x), μ(y))
-- noun, transitive-verb object (already demo-built):
greater : ( ((S\NP)/cat_pp_than) \ ((S\NP)/NP) ) / cat_measure   sem  λμ. λV. λy. λx. gt(μ(x), μ(y))
```

- **Relational** scales (`dependent on`, `dependence on`): `cat_measure / cat_pp_arg`, `deg : Entity →
  Entity → float`; the `on`-PP fills the ground → `cat_measure`. Witnessed: the relational positive
  `dependent on BRCA1` already parses (`gt(deg_dependent(brca1, hela), std_dependent)`); the **only** gap
  is the analytic `more`/`less` operator (§1).
- `more dependent on WRN` and `greater dependence on WRN` route through the *same* `deg_dependent` → the
  same `gt` — identical by construction.

### 5.3 Emission + detection (the importer's job for #8) — core ALREADY BUILT

The count path (#9) is pure grammar. The **degree** path is a lexical **gradable class** emitted by the
importer — and its detection + core emission are **already implemented** in `crates/eigenius-wordnet`
(`2026-07-07`, verified against `a016cea`):

- `wndb.rs` reads the **pertainym (`\`) pointer** → splits **relational** (non-gradable) from
  **descriptive** (gradable) adjectives (`relational` flag).
- `convert.rs::push_adj` (595–683) already emits, per gradable adjective: `axiom deg_X : Entity → float`
  `+ std_X`, the **positive** (`gt(deg_X(x), std_X)`, `S[adj]\NP`), and the **synthetic `-er`
  comparative** (`gt(deg_X(x), deg_X(y))`, `(S[adj]\NP)/cat_pp_than`). The code itself flags the rest as a
  follow-on: *"periphrastic 'more X' … emit only the positive; the `more`/`most` words are a follow-on"*
  (601–602, 666).

So the WordNet method is designed **and its core is built**. #8's remaining work is that documented
follow-on — **incremental on `push_adj`, not undesigned**:

1. **Expose `deg_X` as a `cat_measure` reading** (a bare `deg_X : Entity → float` entry alongside the
   positive `S[adj]\NP`), so the closed-class `more`/`less` (§5.5, Phase B) can operate. Closes
   **non-relational** adjectival comparatives (`more sensitive than Y`) at scale. Small `push_adj`
   addition.
2. **Nominalization projection** — give the deadjectival noun (`dependence`) a `cat_measure` reading with
   `μ = deg_X`, via WordNet **derivational (`+`) links** (already parsed by `wndb.rs`); plus the
   **`attribute` (`=`)** relation for attribute nouns (`weight` ← `heavy`). Closes `greater dependence`.
3. **Relational gradable adjectives** (`dependent on`, `sensitive to`) → `cat_measure / cat_pp_arg[prep]`
   ground form. **DECISION (`2026-07-07`): a curated `adj → prep` map AND an optional-ground type-shift**
   — the two are complementary, not a choice (options (ii) any-PP / (iii) optional-ground were mis-framed
   as alternatives):
   - **Gloss-derived `adj → prep` (WordNet-internal; `2026-07-07`, SUPERSEDES the curated map).**
     WordNet has no subcat frame, but the **gloss** carries governance — extracted by
     `governed_preposition` (convert.rs, built + unit-tested): (1) WordNet's explicit ``followed by `PREP'``
     convention (67 adj synsets, e.g. `proportional`→`to`); (2) the **lemma** immediately followed by a
     preposition in the gloss/examples — lemma-keyed, so it dodges verb+prep noise (`spoke in`) and gives
     the right per-lemma prep *within one synset* (`addicted`→`to`, `dependent`→`on`). `Some(prep)` ⇒
     relational (emit `cat_measure/cat_pp_arg[prep]`); `None` ⇒ bare measure (C1) — so the extractor **IS**
     the relational-gradable detector; no hand-curated list.
   - **Parameterization — DONE as C3-precision (this session), after the milestone.** Making
     `cat_pp_arg` preposition-specific (`cat_pp_arg(prep_on)` vs `(prep_to)`) is what rejects
     `*dependent to`. The importer's PP-oblique **verbs** also emit `cat_pp_arg` from
     **preposition-agnostic** frames (WordNet frame 23 = "`----s` PP"), so the parameterization is a
     `prep` FEATURE with a **wildcard** (verbs → `cat_pp_arg(prep_any)`, gloss-detected adjectives →
     `cat_pp_arg(prep_on)`; `feat_meets` unification) — a new feature dimension parallel to fin/num,
     realized as `data lexicon:Prep` (11 concrete preps + `prep_any`). The generic `cat_pp_arg` (`on_arg`)
     **already threaded the ground faithfully**, so **#8 CLOSED at scale WITHOUT it** (C4) — the prep
     feature only adds the `*dependent to` rejection. Delivered milestone-first (relational emission, C3-wire)
     then the prep feature (C3-precision); the latter ManifestDrifts the snapshot, so its at-scale check
     rides the next reseed.
   - **Null-instantiation via TWO measures, not an `∃`-close shift (`2026-07-07` finding — DONE).** The
     dropped-ground reading (`… proved more sensitive than MSS lines` — `to X` omitted) does NOT need a
     unary type-shift: an `∃`-close over the ground is **ill-typed** (`∃g. deg(g, x)` is not a float, and
     `cat_measure`'s denotation is `Entity → float`). Instead the importer emits **both** measures for a
     relational adjective — the bare 1-place `deg` (via C1 → the ground-less reading `more sensitive than
     Y`) **and** the 2-place `deg_rel` (the `cat_measure/cat_pp_arg` reading → `more sensitive on/to X than
     Y`). Two independent opaque measures (the `∃g` relation between them is deferred, §7). **No parser
     change**; wired in `push_adj` (C3-wire, unit-tested `relational_gradable_adjective_emits_ground_taking_measure`).

- **Operators** (`more/less/greater/fewer/than`) → closed-class (bootstrap), **not** importer (§5.5, Phase B).

**Grounding pass = validation, not design.** Confirm the pertainym split + `deg_X` emission + derivational/
`attribute` pointers give good empirical coverage of the target adjectives/nominalizations. (Correction to
an earlier framing of §5.3 as an "undesigned effort": the mechanism is built; what remains is items 1–3 +
this coverage check.)

### 5.4 The emit-vs-rule fork — resolved, and it splits by mechanism

The fork the note left open (`d63-passive-voice-handling.md`-style) resolves *differently* for the two:

- **#9 cardinality → grammar rule.** `fewer`/`more` over any `cat_n`; no per-noun data, no reseed.
- **#8 degree → importer data.** Gradability, relationality, and the governed preposition are lexically
  idiosyncratic → emit per-adjective, project to nominalizations.

### 5.5 Analytic `more`/`less` — the factoring decision

To let `more`/`less` operate, a gradable adjective must expose its `deg_A` as a handle. Today the slice
bakes the positive (`large`) and the synthetic comparative (`larger`) into separate lexemes that both
reference `deg_large`. Options:

- **(a) Kennedy factoring** — the adjective supplies `deg_A : Entity → float` (a `cat_measure`); *positive*
  (`deg` vs a standard), *comparative* (`more`/`-er`), *superlative* (`most`) are operators over it. Clean,
  unifies pos/cmp/sup, and makes the adjective's `deg_A` literally the object the nominalization shares.
  Bigger change to the predicative slice.
- **(b) surgical** — add `more`/`less` as operators referencing the `deg_A` axiom the existing lexemes
  already carry (`more large` → `gt(deg_large(x), deg_large(y))`). Minimal; closes #8's adjectival route
  without reworking positives.

Recommend **(b)** to close #8 now, **(a)** as the eventual structure.

### 5.6 Cost / ambiguity

- Cardinality: `fewer`/`more` over `cat_n` — bounded (operator-triggered).
- Degree: each gradable adjective gains a `deg` reading and each gradable noun an extra `cat_measure`
  reading — added ambiguity for those words, compounding the mass-shim over-generation that is the Phase-4
  blocker. Mitigate: emit only genuinely-gradable words, rank the extra readings low, lean on the beam.

### 5.7 Shared gaps + placement

**Shared by #8 and #9** (independent of the comparative): the complex subject (`MSI cell lines from these
four lineages`) and possessive/demonstrative than-object (`their MSS counterparts` / `typical lineages`) —
separate NP gaps (determiner + number, possessive), tracked in
[d63-parse-gap-closure.md](d63-parse-gap-closure.md).

**Placement (the lesson from the reverted attempt).** Operators (`greater/fewer/more/less/than`) are
closed-class → **bootstrap**; exemplar gradable **words** (`dependence`, `dependent`) are test scaffolding
→ **demo lexicon** (`experiments/lexicon/lexicon.esl`), **never** bootstrap. A curated measure-noun starter
set placed in `closed-class.esl` and reseeded baked content-word scaffolding into the permanent snapshot
(wrong shape; also fed the mass-shim ambiguity at scale). Reverted to `a016cea`; general emission is the
importer (§5.3), gated on this design.

### 5.8 Correction to the committed `fewer` (`a016cea`)

`a016cea` **mislabels `fewer`.** The demo-lexicon `fewer_cmp` is the **degree-LESS** operator over
`cat_measure` (sem `gt(μ(y), μ(x))`, sense `wn:few.a.01`, "over the same measure machinery") — i.e. it is
**`less`'s** semantics wearing the word `fewer` — and `phrasal_comparative_compares_measure_degrees`
asserts `HeLa affects fewer dependence on BRCA1 than MSH2` **parses**. The design (§4 agreement, §5.0/§5.1)
requires the opposite split:

- **`fewer`** = cardinality over `cat_n` (`fewer mutations`; `*fewer dependence` is **out**);
- **`less`** = the degree-LESS over `cat_measure` (`less dependence`) — which is exactly what `fewer_cmp`'s
  category/sem already is.

**Fix (demo lexicon + test only — no bootstrap):** rename `fewer_cmp` → `less` (its `gt(μ(y),μ(x))` sem is
correct for `less`), add a new `fewer` over `cat_n`, and flip the test — `*fewer dependence` gets **no**
parse; assert `fewer <count-noun>` instead. Until then the committed `fewer` wrongly accepts the
ungrammatical `fewer dependence`. This lands with the #9 cardinality work (§5.1); it is not a revert of
`a016cea` (`greater`, `cat_measure`, the `dependence` scaffolding, and the predicative slice are all kept).

## 6. Anchors (verify DOIs before load-bearing — `note`-flagged)

- **Hackl 2000**, *Comparative Quantifiers* (MIT diss.) — `more`/`fewer` as comparative quantifiers.
- **Bhatt & Takahashi 2011**, *Reduced and unreduced phrasal comparatives*, NLLT — the direct analysis (Q3).
- **Kennedy 2007**, *Vagueness and Grammar*, Ling.&Phil. — gradable degree semantics.
- **von Stechow 1984** / **Heim 2000** — degree-operator scope.
- **Solt 2015**, *Q-adjectives* — `many/few/more/fewer` as quantity adjectives.
- **[added] Morzycki 2009**, *Degree modification of gradable nouns*, Natural Language Semantics 17 — nominal gradability.
- **[added] Constantinescu 2011**, *Gradability in the Nominal Domain* (Leiden diss./LOT).
- **[added] Bale & Barner 2009**, comparatives and the mass/count distinction, J. Semantics — the cardinality-vs-degree comparison.
- **[added] Rothstein 2017**, *Semantics for Counting and Measuring* — the *measure noun = unit* sense (the contrast that makes our term wrong).
- **MTT/DTS gap:** no worked comparative-*quantifier* / gradable-noun account in type-theoretic semantics
  located; the opaque per-dimension `Entity→float` + direct 3-place `gt` is, as far as found, novel for MTT.

## 7. Faithfulness bound (what we commit vs defer)

**Committed (faithful):** `gt(deg(subj), deg(std))` — a correct graded proposition; transitivity + inverse
hold. **Deferred (later refinement, not needed for the claim):** the internal structure of `deg` (its
derivation from the adjective/nominalization; the hypernymic relation between dimensions; the underlying
eventuality). Faithful *enough to commit as a graded claim*, insulated from full compositional degree
semantics — the D61 faithfulness line.

## 8. Implementation Order

## [x] Phase A — Demo mechanism (no reseed; experiments/lexicon/lexicon.esl + kernel/tests) — DONE + committed (`2026-07-07`)
Prove the whole grammar before any bootstrap/importer commitment. Landed: 129 tests green, fmt+clippy clean.
Build notes: the `cat_forall`+`cat_n` parser rule (parser.rs:298) is how an operator consumes a count noun
(binds T, T-free VP → plain Apply); `strip_feature_binders` (lexicon.rs:152) peels only LEADING
fin/num foralls, so the noun-type `cat_forall` must sit UNDER them.

[x] A1 — Fixed the mislabeled fewer (§5.8): demo fewer_cmp → less; phrasal test flipped (*fewer dependence no parse).
[x] A2 — #9 cardinality: fewer/more(count) over cat_n via cat_forall+cat_n; card : Set→Entity→float; denotation + agreement (*fewer dependence GAP) + compound (gene cell lines) tested.
[x] A3 — #8 adjectival more/less over a gradable adjective's deg_A (predicative ((S[adj]\NP)/cat_pp_than)/cat_measure); relational dependent on X (cat_measure/cat_pp_arg). Test X is more/less dependent on Y than Z.
[x] A4 — Noun & adjective share ONE deg_dependent; asserted `more dependent on BRCA1` and `greater dependence on BRCA1` produce the identical gt(deg_dependent(brca1, hela), deg_dependent(brca1, msh2)).

## [ ] Phase B — Promote operators to closed-class + reseed → closes #9 at scale

[ ] B1 — Move operators to closed-class.esl (bootstrap). greater/less/more/fewer + the card functor + scale plumbing. (than/cat_pp_than is already closed-class.) Measure nouns / gradable adjectives stay out (demo/importer).
[ ] B2 — Reseed + verify #9 at scale. #9 operates over existing cat_n, so it closes with just the operators — no importer emission needed. Probe cells contained fewer mutations than genes: GAP→CLOSED. #8 still GAPs.

## [ ] Phase C — #8 degree at scale: importer gradable emission (§5.3) — core ALREADY BUILT
The importer already DETECTS gradable adjectives (`wndb.rs` pertainym `\` split) and EMITS `deg_X` + `std_X`
+ positive + synthetic `-er` (`convert.rs::push_adj`, 595–683). Remaining = the code's own documented
"periphrastic more X" follow-on, incremental on `push_adj` — NOT an undesigned effort. Prereq is a
COVERAGE-VALIDATION grounding pass (not design) + verifying the §6 anchor DOIs.

[x] C1 — DONE (committed cc5236e). Bare deg_X : Entity→float cat_measure reading alongside the positive S[adj]\NP → closes NON-relational adjectival comparatives (more sensitive than Y).
[x] C2 — DONE (committed f8b817a). Nominalization projection: the deadjectival noun (dependence) gets a cat_measure reading, μ = deg_X, via WordNet derivational (+) links; verified at scale in C4 — noun and adjective forms give the byte-identical deg_dep_rel term.
[x] C3-wire — Relational emission DONE (2026-07-07). governed_preposition (gloss-derived: "followed by `PREP'" + lemma-keyed; addicted→to, dependent→on) drives push_adj → relational adjectives emit a 2-place deg_rel + a cat_measure/cat_pp_arg reading + a relational projection onto the nominalization; the bare 1-place deg (C1) covers the dropped-ground reading, so NO parser type-shift is needed (∃-close is ill-typed over a float — 2026-07-07 finding). Unit-tested (relational_gradable_adjective_emits_ground_taking_measure); fmt+clippy clean. #8 EMISSION-COMPLETE; parse mechanism demo-verified (A3/A4). Generic cat_pp_arg (on_arg) threads the ground.
[x] C4 — #8 VERIFIED AT SCALE (2026-07-07, wordnet-umls-all-2026-07-07-c3 snapshot; diagnostic verify_degree_comparative_at_scale). `greater/more dependence/dependent on genes` → identical `gt(deg_dep_rel(genes, cells), deg_dep_rel(genes, mutations))` (C2 shared scale + C3 governed-prep relational reading); `more sensitive than` (WordNet-only adj) → 1-place `gt(deg_sensitive(cells), deg_sensitive(mutations))`; #9 card regression holds — all ⊨Prop. Residual: comparative `than`-attachment (ground vs subject) is enumerated, not disambiguated.
[x] C3-precision — DONE (this session). cat_pp_arg carries a `prep` FEATURE (new `data lexicon:Prep`: 11 concrete preps + prep_any), a new feature dimension parallel to Num/Fin: verbs (prep-agnostic WordNet PP frames) → cat_pp_arg(prep_any), gloss-governed adjectives/nominalizations → cat_pp_arg(prep_X) from governed_preposition; unified via feat_meets (wildcard meets any, specific meets equal). 11 closed-class arg-markers tagged (to/from/on/with/in/for/at/upon/about/against/into). Packing stays sound (cat_shape keeps feature ctors, like num/fin). Differential test governed_preposition_gates_the_oblique_pp: `more dependent ON` parses, `*more dependent TO` rejected. 1611 kernel + 130 comparative tests + fmt + clippy clean. **AT-SCALE VERIFIED (`2026-07-07`, WordNet `--all` native load, 616 k resources; diagnostic `verify_governed_preposition_at_scale`).** Refinement from the at-scale run: `*dependent to` is NOT a full-sentence GAP at scale — WordNet gives `dependent` a bare `cat_measure` (C1) + a count-noun reading (`cat_n(wn:n10004804)`) that close the sentence regardless of the preposition. C3-precision's guarantee is on the **relational reading** specifically: the ground-taking `deg_a00725772_rel(ground, subject)` term (built only through `cat_measure/cat_pp_arg(prep)`) appears with the governed prep (`on` → 4 relational parses) and is **absent** with the wrong one (`to` → 0 relational). Precision on the forest, not on closure. Importer emission confirmed at scale: verbs→prep_any (7398), gloss-governed adjectives→specific preps (prep_on 85, prep_to 1179, prep_in 952, …). Full reseed (WordNet+UMLS, domain-entity #8) still blocked on the load OOM (reseed-oom-memory-investigation.md), decoupled from this witness.

## [ ] Phase D — Shared NP-complexity gaps (§5.7; independent, interleavable)
Needed for the full corpus sentences, independent of the comparative: complex subject (MSI cell lines from these four lineages — modifier + plural compound + these four demonstrative+cardinal) and possessive than-object (their MSS counterparts). Separate gaps (determiner+number, possessive), tracked in d63-parse-gap-closure.md.

## [ ] Phase E — Deferred: Kennedy factoring (§5.5(a))
Refactor the predicative slice to deg_A + pos/cmp/sup operators (the eventual structure). Not needed to close #8/#9.

Corpus-sentence closure: #9's full sentence closes after B2 + D; #8's after C4 + D.

