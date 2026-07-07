# D63 — Phrasal comparatives (`greater/fewer X … than Y`): grounded design

**Status:** grounded design (expert-consulted `2026-07-06`) → **demo build DONE + verified** (§6); the
snapshot #8/#9 close is pending measure-noun seeding + a reseed (§7). The RC-2 phase-2 gap
([d63-parse-gap-closure.md](d63-parse-gap-closure.md) roadmap). Started life as the *question* artifact
(the five open discovery targets); the expert's answers discharge them and are mapped back below as the
design. The **faithful** treatment (option b) proved **tractable**, not research-grade — the expert
collapsed the uncertainty (esp. Q2/Q5: an opaque per-dimension measure suffices).

## 1. Witnessed facts (Derived, `2026-07-06`, snapshot `wordnet-umls-all-2026-07-06`)

`probe_rc2_comparatives`:
- **Attributive comparatives already parse:** `a stronger phenotype affects cells` CLOSED×168,
  `greater dependence affects cells` CLOSED×72 — via the *positive* `S[adj]\NP` reading of `great`/`strong`
  (morphy lemmatizes `greater`→`great`) riding the attributive-Σ rule (Slice 3b).
- **The `than`-clause is the gap:** `WRN showed greater dependence than genes` GAP,
  `cells contained fewer mutations than genes` GAP. `cat_pp_than` is consumed only by the *predicative*
  comparative functor `(S[adj]\NP)/cat_pp_than`; with `greater` already attached attributively, the
  `than`-phrase has nothing to bind to.
- **Scope:** the two real RC-2 gaps are #8 (`greater dependence on WRN than their MSS counterparts`) and
  #9 (`fewer deletion mutations … than typical lineages`). **#12 is misfiled** — its attributive
  comparative parses; #12 gaps on the modal `may require` + compound subject `WRN dependency`.
- **No reference grammar helps:** core-en treats comparatives as ordinary `n/n` adjectives (no `than`/
  degree); lightblue's English lexicon is a 99-line POS stub (JJR unmapped, no `than`). Eigenius's
  predicative degree slice (`deg_A`/`measurements:gt`, D63 §8.12) already exceeds both.

## 2. The five questions → the grounded design (expert consultation, `2026-07-06`)

Each was an open discovery target (Declared); the expert's answer discharges it. Answers are **grounded**
(expert judgment + cited literature — anchors in §4, DOIs `note`-flagged pending verification).

- **Q1 — comparative QUANTIFIER, not gradable adjective.** `greater` = mass quantifier (≈ "more"),
  `fewer` = count quantifier — measuring the AMOUNT/CARDINALITY of the noun's extension, NOT the degree of
  "great"/"few". So the predicative slice does **not** generalize, and the `greater→great` lemmatization is
  **actively harmful** (it asserts a different proposition). Theoretical measure:
  `μ_amount : (Entity → Prop) → float` (over a predicate extension).
- **Q2 — STIPULATE the measure, don't derive it.** Full compositional derivation would force reifying
  events/sets in the type theory. In an MTT with only `float`+`gt`, the cleanest object collapses the
  predicate-measure to a per-**dimension** entity measure: **`μ_dep_on_WRN : Entity → float`** — maps the
  cell line directly to its value on the "dependence on WRN" scale, bypassing reifying the abstract amount.
  (This is the pivotal tractability move: same type shape as the existing `deg_A`.)
- **Q3 — the DIRECT / phrasal analysis suffices** (Bhatt & Takahashi). The contrasts are subject-vs-subject
  (`MSI lines` vs `MSS counterparts`), so a **3-place comparative combinator** avoids clausal-ellipsis
  resolution: **`λμ. λy. λx. gt(μ(x), μ(y))`**. Reduced-clausal is needed ONLY for subcomparatives
  (`longer than the door is wide`) or adjunct contrasts (`dependence on WRN than on BRCA1`) — not here.
- **Q4 — CCG attachment via the comparative-scope maneuver.** `than Y` attaches to the **VP/S, not the
  object NP**. The comparative object NP type-raises to a GQ that consumes the transitive verb and passes a
  **`[comp]` feature** up to the VP, leaving a λ-abstracted `μ`; the `than`-phrase is a VP-adjunct
  `(S\NP) \ (S\NP)[comp]` that consumes `μ` and applies it to both the standard it contains and the subject
  it meets. Object NP category ≈ `(S\NP) \ ((S\NP)/NP)` + `[comp]`.
- **Q5 — an opaque per-dimension `μ` is faithful for a graded KG.** The line is **intra-measure
  entailment**: opaque `μ` gives transitivity (`A>B ∧ B>C → A>C`) and inverse (`A>B → B<A`) for free. What
  it can't do without extra axioms — decompose `μ_dep_on_WRN` vs `μ_dep_on_BRCA1` via a hypernymic
  "dependence", or infer a depending-event exists — is **not needed** to extract the graded claim.
  `gt(μ_dep_on_WRN(MSI), μ_dep_on_WRN(MSS))` captures the sentence's informational payload.

## 3. The design (to implement)

1. **Measure object:** an opaque per-dimension measure `μ_dim : Entity → float` (same shape as `deg_A`),
   keyed to the nominal+PP dimension. **Open Eigenius decision:** stipulate atomically per (noun, pp-object)
   — unscalable — vs **parameterize**: a measure-noun carries `μ_noun : Entity → Entity → float`
   (pp-object, subject → float); `on WRN` supplies WRN → `μ_noun(WRN) : Entity → float`. Parameterized (ii)
   is scalable (one `μ` per measure-noun) and is the recommended realization.
2. **`greater`/`fewer` as comparative determiners / Q-adjectives** (Solt's Q-adjectives), NOT the positive
   adjective. They build the object-GQ form carrying `μ` + `[comp]`. Suppress/deprioritize the harmful
   `greater→great` positive reading for the quantity sense (a sense-rank or an importer emission).
3. **`than` marker** — reuse `than_marker`/`cat_pp_than` (already exists), but the `than`-phrase now
   attaches as the VP-adjunct `(S\NP)\(S\NP)[comp]`, consuming the `[comp]`-passed `μ`.
4. **Comparative combinator sem:** `λμ. λy. λx. gt(μ(x), μ(y))` (direct 3-place).

**Open Eigenius-specific implementation decisions:**
- **Importer-emit vs grammar rule** — emit the comparative-quantifier entries per measure-noun (data), or
  a grammar type-changing rule that lifts any measure-noun (generalizes without re-seeding). Mirror the
  `d63-passive-voice-handling.md` importer-vs-rule choice.
- **The `[comp]` feature** — a new `Fin`/feature value vs reuse. Threads the measure from the object NP up
  to the VP for the `than`-adjunct.
- **Which nouns are measure-nouns** — `dependence`, `mutations`, … carry `μ`. A lexical class (importer)
  vs any `cat_n` (over-general). Ties to the count (`μ_card`) vs mass (`μ_amount`) split (Q1).

## 4. Anchors (verify DOIs before load-bearing — `note`-flagged)

- **Hackl 2000**, *Comparative Quantifiers* (MIT diss.) — `more`/`fewer` as comparative quantifiers (Q1).
- **Bhatt & Takahashi 2011**, *Reduced and unreduced phrasal comparatives*, NLLT (Q3, the direct analysis).
- **Kennedy 2007**, *Vagueness and Grammar*, Ling.&Phil. — gradable degree semantics.
- **von Stechow 1984** / **Heim 2000** — degree-operator scope.
- **Solt 2015**, *Q-adjectives* — `many`/`few`/`more`/`fewer` as quantity adjectives (design point 2).
- **MTT/DTS gap:** no worked comparative-*quantifier* account in type-theoretic semantics located; this
  design (opaque per-dimension `Entity→float` + direct 3-place `gt`) is, as far as found, novel for MTT —
  the expert flagged it as the real open area.

## 5. Faithfulness bound (what we commit vs defer)

**Committed (faithful):** `gt(μ_dim(subj), μ_dim(std))` — a correct graded proposition; transitivity +
inverse hold. **Deferred (later refinement, not needed for the claim):** the internal structure of `μ` (its
derivation from the nominalization; the hypernymic relation between dimensions; existence of the underlying
depending-event). This is the D61 faithfulness line — faithful *enough to commit as a graded claim*,
insulated from full compositional degree semantics.

## 6. Demo build — DONE + verified (`2026-07-06`)

The mechanism is built and green on the demo grammar (no reseed), exactly on the approved design:
- **`cat_measure`** added to `lexicon:Cat` (⟦·⟧ = `Entity → core:float`, `denote_cat` arm); the
  `N[measure]` restriction falls out of the category (only measure nouns produce `cat_measure`).
- **Measure noun** `dependence` = `cat_measure / cat_pp_arg`, sem `μ_dependence : Entity → Entity → float`
  (ground, subject); `on WRN` (the existing `on_arg`) fills the ground → `μ_dependence(WRN)`.
- **`greater`** = `( ((S\NP)/cat_pp_than) \ ((S\NP)/NP) ) / cat_measure` (fin/num threaded like `a_obj`),
  sem `λμ.λV.λy.λx. gt(μ(x), μ(y))`. **`[comp]` = the `/cat_pp_than` on the object-GQ's result** (reuses
  the predicative-comparative mechanism; no new feature). **`fewer`** = the LESS variant `gt(μ(y),μ(x))`.
- **Verified** (`closed_class_determiners::phrasal_comparative_compares_measure_degrees`):
  `HeLa affects greater dependence on BRCA1 than MSH2` → `gt(μ_dependence(brca1)(hela),
  μ_dependence(brca1)(msh2))`, type-checks to `Prop`; `fewer` parses; `*greater gene` (non-measure noun)
  rejected. Full demo suite 127 + kernel lib 1611 green, fmt + clippy clean.

## 7. Snapshot path (#8/#9) — remaining

To close #8/#9 over the full lexicon, three pieces + a reseed:
1. **`greater`/`fewer` → `closed-class.esl`** (truly closed-class; small; moves them off the demo fixture).
2. **Measure-noun seeding — the open "which nouns" boundary** (the expert's buckets: event/state
   nominalizations `dependence`/`expression`/`growth`; relational quantities `number`/`rate`/`level`).
   Each needs `cat_measure/cat_pp_arg` + a per-noun `μ` axiom. **Options:** a curated starter set in
   `closed-class.esl` (covers the document; fast) vs importer-emitted measure-noun class (general;
   bigger). #9's measure is a **compound** (`deletion mutations`) + PP — needs the compound-measure
   handling (`cat_measure` analogue of `RefineKind::KindCompound`), a step beyond #8's simple `dependence`.
3. **Reseed** (~40 min) then verify #8/#9 + re-measure.
