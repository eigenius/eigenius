# D63 — Collapsing the domain-compound pile (plan of attack for the 3 residual gaps)

**Status:** plan / pre-implementation. Targets the 3 residual reranked gaps (#3 passive, #4 V-as-Y +
compared-to, #7 comparative + PP) that survive after lexicalize + build-then-subsume + reranker +
count-veto. Grounded in the re-assessment (`diagnose_residual_gaps`, `db_backed_encoding.rs`) that
**refuted PP-attachment as the lever** ([d63-pp-attachment-control-scoping.md](d63-pp-attachment-control-scoping.md),
shelved) and located the driver in **domain-term ambiguity — sense-product × N-N compound bracketing**.

## 1. What we know (grounded)

- **Grammar is complete for these three.** Every construction fragment parses in isolation with generic
  fillers — the passive + coordinated subject (`some lines and some lines were represented by data sets`
  CLOSED×6), V-as-Y + both PPs (`…as a dependency in cells compared to lines` ×121), comparative + PPs
  (`cells from lineages showed greater dependence on genes than counterparts` ×162). Adding a PP *raises*
  the reading count, never gaps. The gap appears **only** when generic fillers are replaced by the domain
  terms (`MSI cell lines`, `MSS counterparts`, `screening data sets`, `WRN`, `these four lineages`).
- **The pile is ~6 structural shapes × sense-product per shape**, not one or the other (v3 S5
  `analyze_chart_cells`: the saturating cells are `kept=432 shapes=6`, `kept=184 shapes=6`). So there are a
  handful of distinct `cat_shape`s (structural), each holding a large sense-product (same-shape).
- **The two collapse mechanisms already built hit different halves.** Packing
  ([packed-forest blueprint](d63-packed-forest-parsing-blueprint.md), default on) collapses the
  *same-shape* sense-product to O(nodes) (~8× measured) but **not** the distinct shapes. Build-then-subsume
  (D3) drops definitionally-equal readings post-felicity. Neither collapses distinct *structural* shapes.
- **The explosion is the CKY cross-product across the compound spans** (items²-per-split), amplified by the
  domain terms' sense-product and multi-noun brackets — 10.8M chart items on #7, OOM at cell_beam=1024.

## 2. Step 0 RESULT — the corpus NEVER packs (Derived, `2026-07-09`, `diagnose_compound_pile`)

`parse_needs_unpacked` routes the **whole** sentence off the packed path if any seeded span has a
**concrete non-Entity selectional slot** (`cat_np(SpecificClass)`, `slot_is_concrete_nonentity`) or is
pied-piping. **Measured (`routes_packed` over the count-veto snapshot): EVERY frame routes UNPACKED —
including trivial `genes affect cells`, `genes are large`, `genes are attractive targets`, and all three
residual sentences and their generic bases.** So it is **not** the comparative/passive/V-as-Y constructs —
**on the full lexicon the packed path is never taken at all.**

**This is the headline finding, and it reshapes the plan.** The measured ~8× packing win
([blueprint §10b](d63-packed-forest-parsing-blueprint.md)) was validated on *small-lexicon demo*
sentences; on the real corpus the dense lexicon means some sense of some common word always carries a
concrete selectional slot, and the router's **whole-sentence** rule unpacks everything on that single hit.
So the sense-product piles are **never** collapsed by packing — which is why even the generic bases run at
×121–162 and the domain terms tip them over. `self.packing` is on; the router is the sole cause
(`routes_packed = packing && !combinatory_core && !parse_needs_unpacked`).

**Consequence: Lever 1 (fix the whole-sentence router → per-cell packing) is THE lever — and it is
corpus-wide, not just the 3 gaps.** It would recover the packing win for *every* corpus sentence, collapsing
the sense-product piles that drive both the residual gaps and the high AMBIG generally. Lever 2 (collapse
structural shapes) drops in priority — packing is not even running, so "the residual is 6 structural shapes"
was a false premise; the real residual is the *uncollapsed sense-product on the unpacked path*.

## 3. Levers (Step 0 confirmed UNPACKED → Lever 1 is THE lever)

### Lever 1 — Per-cell packing (CONFIRMED corpus-wide) ★
Today `parse_needs_unpacked` is **whole-sentence**: one selectional slot anywhere unpacks everything,
including the index-independent noun-compound sub-cells that are the actual pile — and Step 0 showed that
on the full lexicon this fires on *every* sentence, so packing never runs. **Fix: pack the safe sub-cells,
unpack only the slot-bearing spans** — the noun compounds (`MSI cell lines`, `screening data sets`) carry
no selectional slot and are soundly packable even inside a sentence whose verb selects. Recovers the 8× on
exactly the spans that explode, on every corpus sentence.
- **Step 1 RESULT (Derived, `2026-07-09`, `EIGENIUS_ROUTE_DEBUG` on `diagnose_compound_pile`).** The
  offending category is the **same on every noun** — the object-position type-raised existential-GQ seeded
  on the bare plural (via the existential det-form, `lookup.rs:855`):
  `(S\NP)\((S\NP)/cat_np(<the noun's own synset>))` — e.g. `genes`→`…/cat_np(n05436752)`,
  `cells`→`…/cat_np(n00006484)`, `lines`→`…/cat_np(n00582388)`. Its argument slot is the noun's **own
  concrete class** (not `Entity`), so `cat_has_selectional_slot` fires and the whole sentence unpacks.
  Every noun carries it (object-position quantified NPs — `affect genes`, `represent lines` — need it), so
  **every** sentence trips the whole-sentence router even when the object-GQ reading is never used (`genes`
  is the *subject* in `genes affect cells`).
  - **Verdict: legitimate slot, not a spurious sense — so Lever 1 is per-cell packing, NOT a
    source/router-precision tightening.** The concrete slot is *semantically load-bearing*: it records
    which class fills the object (so `affect genes` denotes gene-object semantics), and combines with the
    generic verb `(S\NP)/cat_np(Entity)` only by contravariant subsumption (`gene ⊑ Entity`). Widening it
    to `Entity` would erase the object's type; it can't be tightened away. Packing by `cat_shape` (which
    erases `cat_np(gene)`→`cat_np(_)`) is therefore genuinely unsound *for the object-GQ item* — two nouns'
    object-GQs share a shape but combine/denote differently. The router is right to distrust it; it is
    **wrong to unpack the whole sentence** over it.
  - **So the object-GQ is a small, per-cell *unpacked residue*, not the pile.** Within each cell, the
    index-**in**dependent items — the plain NP, the compositional compound readings, the whole sense-product
    that is the actual explosion — are soundly packable; only the handful of concrete-slot object-GQ (and
    pied-piping) items must stay unpacked. Per-cell packing packs the pile and unpacks the residue. The fix
    is exactly Lever 1 below.
- Touches: `parse_needs_unpacked` (per-cell, not per-sentence); the packed-forest construction to mix
  packed sub-cells with unpacked slot-spans; the differential oracle (extend to mixed sentences).
- Risk/cost: the packed/unpacked boundary bookkeeping is the real work; the soundness precondition
  (index-independence of the packed sub-cells) is already the packing invariant, so no new unsoundness.

- **Step 2 — IMPLEMENTED (Derived, `2026-07-09`).** The fix is cleaner than "mix packed and unpacked
  sub-cells": per-cell packing falls out of a **packing-signature refinement**, so there is one packed
  forest, not a packed/unpacked split.
  - `node_sig` (`packed.rs`) now keys an item by `cat_shape` (indices erased — the coarse key that
    collapses the sense-product) **unless** its category has a concrete selectional slot
    (`cat_has_selectional_slot`), in which case it keys by the full category (`cat_key`, new in
    `pretty.rs`, prefixed `sel:`). So two object-GQs of different classes never share a node; the
    index-independent majority (the actual pile) still packs by `cat_shape`. The object-GQ is the small
    per-cell unpacked residue, exactly as Step 1 predicted.
  - The router's whole-sentence selectional carve-out (`parse_needs_unpacked` clause 2) is **removed** —
    concrete slots are sound per-cell now. Only the pied-piping **completeness** carve-out remains (the
    packed forest builds no edge for that ternary construct).
  - Removing the carve-out exposed one construct the selectional carve-out had incidentally been
    protecting: **close nominal apposition** (`the genes BRCA1 and MSH2`), which the packed forest did
    not build. Built it in as an `ApposeGroup` binary edge over adjacent splits (mirrors the unpacked
    CKY) — the structural completion, not a re-carve-out.
  - **Soundness witnessed** by the differential oracle `packed_forest_equals_unpacked_on_core_grammar`,
    extended with selectional (`depends on`, object-GQ) and close-apposition sentences: packed ≡ unpacked
    (closed forests + open counts) on all of them. Full kernel suite green (1605 + 135), `fmt`/`clippy`
    clean.
  - **Corpus witness — routing:** every corpus frame now routes `[PACKED]` (was `[UNPACK]` on every
    frame at Step 0), including all three residual-gap sentences.
  - **Corpus witness — deterministic cap-only sweep** (count-veto snapshot, no reranker): grammar-gap
    **7 → 2**, **zero new gaps** (strict subset — no regression), **5 closed** (incl. the #3 data-sets
    sentence, the #7 `greater dependence on WRN` sentence, `These observations suggest…`, `WRN dependency
    may require…`, `We hypothesized… synthetic-lethal`). The #7 sentence had previously OOM'd at
    `cell_beam=1024`; packing collapses the sense-product so it parses. Remaining 2 gaps: the
    `Project Achilles and project DRIVE identified WRN as…` sentence and `The MSI relationship compared
    favourably…`.
  - **Corpus witness — reranked sweep** (`--features use-llm`, non-deterministic): grammar-gap **3 → 1**
    (closed `MSI cell lines from these four lineages showed greater dependence on WRN than their MSS
    counterparts` and `Some MSI lines and some MSS lines were represented by these screening data sets`).
    The `encoded 1 → 0` / `open 0 → 1` shifts are within the reranker's per-unit non-determinism (the
    deterministic cap-only sweep above is the no-regression proof), not lost readings.
  - **Remaining gap (both sweeps agree):** `Project Achilles and project DRIVE identified WRN as the top
    preferential dependency in MSI cell lines compared to MSS cell lines`.

## 7. Remaining gap DIAGNOSED — it is a grammar gap in the `as`-predication complement, NOT search (Derived, `2026-07-10`, `diagnose_project_achilles`)

The "search-limited on a triple-compound clause" label above is **refuted**. Isolating the one sentence
by swapping each feature of the generic base in one at a time (`diagnose_project_achilles`,
`db_backed_encoding.rs`) shows every domain compound composes cleanly and the whole thing gaps on a
**single phrase**:

| swap into the generic base | result |
|---|---|
| subject → `Project Achilles and project DRIVE` | CLOSED×112 |
| object → `WRN` | CLOSED×112 |
| `in cells` → `in MSI cell lines` | CLOSED×96 |
| `compared to lines` → `compared to MSS cell lines` | CLOSED×216 |
| `a dependency` → **`the top preferential dependency`** | **GRAMMAR-GAP** |

It gaps **even at `cell_beam=1024`** (WIDE), so it is not beam pressure. Drilling into the phrase as the
`as`-complement (`identify X as Y`) vs plain object position:

| `identified genes as …` | | `genes affect …` (object) | |
|---|---|---|---|
| `a dependency` | CLOSED | `a top preferential dependency` | CLOSED×12 |
| `a top dependency` | CLOSED | `the top preferential dependency` | CLOSED×9 |
| `the dependency` | **GAP** | `a preferential dependency` | CLOSED×9 |
| `a preferential dependency` | **GAP** | `a top dependency` | CLOSED×33 |

The NP parses in every form in **object** position; only the **`as`-complement** rejects it, on two
independent triggers, both inside `as …`:
1. the **definite article** — `as a dependency` closes, `as the dependency` gaps;
2. the **subsective adjective** `preferential` — `as a top dependency` closes, `as a preferential
   dependency` gaps (`top`, a non-refining modifier, is fine).

**Mechanism (CORRECTED, Derived from `debug_form_entries` + the winning sem — supersedes the
predicate-nominal reading first drafted here).** There is **no functional `as`** in the grammar at all.
`as` seeds only as WordNet/UMLS **nouns** — `As` = arsenic (`n14629149`), a place name (`n08552138`), and
UMLS concepts — no preposition, no predicativizer, no `cat_pp_arg`. The reason `we identified genes as a
dependency` "closed" is a **spurious noun compound**: its winning sem is
`v00618878_t(kind_of(ΣG#0:n14001348. compound_kind(G#0, ΣG#1:n05400860. compound_kind(G#1,
ΣG#2:n14629149. compound_kind(G#2, n05436752)))), speaker)` — i.e. `identify` used **transitively** over the
compound `[dependency [a(letter) [as(arsenic) [genes]]]]`. So the ladder rungs marked CLOSED were garbage
noun-compound readings, never an `identify X as Y` predication. The "gaps" are exactly the cases where the
spurious compound **can't** form: a definite `the` and a subsective adjective `preferential` are not
noun-compoundable, so no reading survives at all. (This also means the earlier #4 ladder "CLOSED×112" was
spurious; the construction never parsed.)

**core-en comparison.** OpenCCG's `core-en` (`references/openccg/grammars/core-en`) has a dedicated
**Predicative** preposition family (`pp.xsl`, `$P.Default.Fig.X.Ground.Y`) — `as`-style predication is a
first-class preposition taking a predicate ground. So the construction is standard and belongs in the
grammar; we simply never added a functional `as`.

**Fix scope (a real grammar-construction addition, not the small lexical tweak first scoped; needs a
reseed).**
- **(1) Add a functional `as`** — the essive / predicative-complement marker — to
  `ontologies/lexicon/closed-class.esl`. Following the existing argmarker pattern (to/from/on/… =
  `cat_pp_arg(prep)/NP`, transparent sem), `as` = `cat_pp_arg(prep_as)/NP` reaches the referential-NP and
  raised-GQ complement forms uniformly (so it handles `as the …`, `as a preferential …`, `as WRN`).
- **(2) A verb frame that consumes it** — `((S\NP)/cat_pp_arg(prep_as))/NP` for the `identify / regard /
  describe / classify / define … X as Y` verbs (the object+PP follow-up already flagged at
  `crates/eigenius-wordnet/src/convert.rs:254`). Needs the frame → category mapping in `convert.rs` and a
  reseed for the corpus (closed-class + importer both change).
- **(3) Suppress the spurious noun-compound** reading that let `as`(arsenic)/`a`(letter) join compounds —
  an ambiguity/precision follow-up, separate from closing the gap. **TRIED AND REVERTED** (see below).
- Not search-related; **Levers 2/3 are not the lever for this gap.** The packing win (§3, GAP 7→2 / 3→1)
  stands independently.

**IMPLEMENTED (Derived, `2026-07-10`; approach = subcategorized + curated verbs, per the design choice).**
- `prep_as` added to the `Prep` enum (`ontologies/lexicon/lexicon-ontology.esl`).
- Functional `as` = `fwd(cat_pp_arg(prep_as), cat_np(Entity, num_any))`, transparent `argmarker_sem`
  (`ontologies/lexicon/closed-class.esl`) — mirrors to/from/on/….
- `FrameKind::Essive` = `((S\NP)/cat_pp_arg(prep_as))/NP`, an opaque 3-place `Entity→Entity→Entity→Prop`
  axiom, emitted **additively** for a curated essive-verb set (`ESSIVE_VERBS` / `is_essive_verb`,
  `crates/eigenius-wordnet/src/convert.rs`) — WordNet's frame inventory has no `as`-complement, so the
  frame is added per-lemma, not by `classify`. High-frequency-noun verbs (`class`/`see`/`use`/`treat`)
  are deliberately excluded (over-generation vs a dominant noun sense).
- Unit tests `essive_verb_emits_object_predicative_as_frame` / `non_essive_verb_gets_no_as_frame`;
  full wordnet suite green; clippy clean. Reseeded (native jemalloc, 6.27M UMLS entries, count-veto
  sanity intact).

**Validation (Derived, fresh reseed).** The corpus sentence now parses as a **real essive predication**,
not a compound: `we identified WRN as the top preferential dependency in MSI cell lines compared to MSS
cell lines` → `v00618878_as(C1337007 [=WRN], Π…gt(deg_…) [top/preferential], speaker) ∧ prep_in(…MSI cell
lines…) ∧ prep_to(…MSS cell lines…)`. **Deterministic cap-only sweep: grammar-gap 2 → 1, zero new gaps**
(the Project-Achilles sentence closed; the one remaining gap — `The MSI relationship compared favourably
to other strong biomarkers for vulnerabilities` — is a different construction, intransitive
`compared favourably to`, unrelated to `as`). Full deterministic progression: **7 (pre-packing) → 2
(packing) → 1 (essive)**.

**Known residue (follow-ups, not the corpus gap).** (a) `we identified genes as the dependency` /
`… as a preferential dependency` still gap — a narrower bare-plural-**object** interaction (a name object
`WRN` closes; a bare-plural `genes` object does not yet), separate from the closed corpus sentence.
(b) The spurious `as`(arsenic)/`a`(letter) noun-compound reading still competes for `as a dependency`
(fix (3) above).

**Fix (3) — glue-word content suppression: TRIED AND REVERTED (Derived, `2026-07-10`).** Deleting the
content-noun entries for function-word surfaces (`as`=arsenic, `a`=letter, …; a curated ~50-word list, a
per-lemma/per-form skip in both importers, 119 entries removed of 6.7M) killed the spurious compound as
intended — but on a reseed it **regressed the deterministic sweep 1 → 5**. Two mechanisms, both witnessed
(`diagnose_project_achilles`, `dump_as_cats` on the glue store): the spurious compounds were (i) *masking
real, pre-existing gaps* — `Project Achilles` is `[cat_n Project][cat_np Achilles]`, a name head that
doesn't form a constituent (`Project Achilles affects cells` GAPS while `project DRIVE affects cells`
CLOSES), and `We evaluated MSI as…` uses `evaluate`, not in `ESSIVE_VERBS`; and (ii) *propping up
cap-fragile generic parses* — `lines were represented by sets` CLOSES but `… by data sets` GAPS after the
cut, though neither `data` nor `sets` is glue (the changed sense-product pushes the reading past the beam).
Decisive point: the benefit is ~nil on the real page — the actual corpus sentence closes via the essive
regardless (arsenic/letter can't compound into the definite, adjectivally-refined `the top preferential
dependency`), so the compound only ever won on the synthetic `genes as a dependency` fragment.
**Disposition: reverted; sense selection is the reranker's job. If we ever want the suppression, do it
coverage-preservingly (worst-rank the glue content sense so the cap deprioritizes it) rather than by
deletion.** The `Project Achilles`-subject gap (`Project <Name>`) and the bare-plural-essive-object gap
(a) are the real follow-ups this unmasked.

## 8. Follow-ups CLOSED (Derived, `2026-07-10`)

**#3 — `compared favourably to` (the last page gap) → the whole page now parses (GAP 1 → 0).** The
diagnosis reframed twice: not the comparative (`cells compared favourably to lines` parses), not the
definite subject (`the gene compared to cells` parses) — the culprit is **`biomarkers`**, a UMLS-only
plural whose singular `biomarker` is not a WordNet lemma. Morphy reduces only to WordNet-KNOWN lemmas, so
`biomarkers` yielded only the exact surface, which the seeder tags SINGULAR (`num == surface ⇒ sg`,
`lookup_span`) — a bare singular count noun cannot take the bare-plural kind shift, so `genes affect
biomarkers` gapped while `genes affect markers` (multi-sense, WordNet) parsed. **Fix:
`Lemmatizer::regular_plural_stem`** — a trait method (default `None`, so the no-morphology `Identity`
baseline is untouched — `does`↛`doe`) overridden by Morphy with `morph.c`'s `-ies→-y` / `-s` detachment
*without* the `is_defined` WordNet gate; `candidate_lemmas` offers the stem so a full-lexicon entry for the
singular gets a PLURAL reading and shifts. Deterministic sweep **GAP 1 → 0**; kernel + lemmatizer unit
tests green; **no reseed** (the `biomarker` singular entry already exists). Progression: **7 (pre-packing)
→ 2 (packing) → 1 (essive) → 0 (domain-plural)**.

**#1 — sortal + proper name (close naming apposition) → implemented.** `Project Achilles` =
`[cat_n Project][cat_np Achilles]` matched no compose rule (NamedCompound is `[cat_np][cat_n]`, the other
order), so it gapped; the full sentence "parsed" only via a spurious `compound_kind` chain over `project`
+ UMLS senses. **Fix:** a sem-blind `combinable` case `[cat_n Sortal][cat_np Name]` → `SemRecipe::Name`,
building the coining reading `kind_of(Σx:Sortal. named(x, name))` — a new opaque axiom
`ontology:named : Entity → Entity → Prop`, the name's referent as the naming TOKEN (so a name whose
lexical sense is unrelated — `Achilles` the hero — still felicitously names a Project; NOT type-checked,
unlike `appose_group`). Result `cat_np(Entity, sg)`, a bare proper-name NP. Rides the existing
`apply`/Combine machinery (both paths) — no packed edge. `Project Achilles affects cells` now parses
CLEAN (`kind_of(Σx:project. named(x, Achilles))`) and coordinates with plain names (`Project Achilles and
BRCA1 affect HeLa`). Kernel suite + a demo test green, clippy clean, deterministic sweep still GAP 0.
- **Residual (not the naming rule): `project DRIVE` is a common noun, not a proper name.** `DRIVE` seeds
  only as WordNet `drive` (`cat_n`), so `project DRIVE` is a compound, not a named individual, and can't
  coordinate with `Project Achilles` (`cat_np`) — so the full sentence still wins the spurious compound.
  `DRIVE` is a coined project acronym; the faithful fix is a **document glossary** entry (`DRIVE` → a
  named Project, making it `cat_np`), the same licensed path as arsenic — NOT a generic
  all-caps→proper-name heuristic (which would collide with the already-handled domain acronyms).

### Lever 2 — Collapse the residual structural shapes (if PACKED, or as the second pass)
The ~6 shapes are the structural variants packing can't merge. Expected sources (confirm in Step 0):
- **(a) unit-vs-compositional.** A domain compound that is *also* a lexicon unit (`cell line`, `data set`
  are UMLS/WordNet units) parses BOTH as the unit AND as `[cell][line]` compositional → distinct shapes.
  **Fix: prefer the lexicon-unit reading** — the same lexicalization principle as hyphenate/inject
  (d63-nominal-modification §4). A multiword-unit span, when present, suppresses the compositional
  re-bracketing. Highest-leverage if Step 0 confirms it dominates.
- **(b) compound nesting.** The left-branching NF (`is_compound_refined`, `parser.rs:392`) forces
  left-branching for the *head*, but the v3 S5 profile still showed single-vs-nested `compound_kind`
  variants. **Fix: tighten the NF** so a 3+-noun compound collapses to exactly one tree (extend the
  existing guard; this is the *N-N* residual, distinct from the §3.3 adjective interleaving that was a
  no-op).
- Touches: `parser.rs` compound rule + `is_compound_refined`; the MWE-vs-compositional seeding in
  `lookup.rs` (§8.4) for (a).

### Lever 3 — Beam headroom (the marginal closer)
The generic fragments already run at ×121–162 — right against `DEFAULT_FOREST_CAP` (256) — so the domain
mass tips them over. After Levers 1/2 thin the pile, a small headroom bump likely closes the residual.
- Options: a targeted `cell_beam` raise on compound-heavy spans (adaptive, not global); or extend the
  widen-on-failure escalation ladder; or a modest `DEFAULT_FOREST_CAP` bump. **Do this last** — raising
  the beam before thinning the pile just moves the OOM.
- Touches: `with_cell_beam` / the widen ladder / `DEFAULT_FOREST_CAP` (`lookup.rs`).

## 4. Sequencing

1. **Step 0** — confirm routing + shape profile per sentence (cheap, bounded; no code).
2. **The dominant lever** — Lever 1 if unpacked, Lever 2(a) if packed-and-unit-driven. One at a time,
   re-measure after each.
3. **Lever 2(b)** — tighten the N-N nesting NF if shapes remain.
4. **Lever 3** — beam headroom to close the marginal tail.

## 5. Verification (per lever)

- **Deterministic:** `diagnose_residual_gaps` — the 3 full sentences move GAP → CLOSED/open at the default
  beam (the fragments already parse, so any change is the domain-term pile shrinking, not grammar).
- **Full page:** cap-only sweep (no-regression diff vs the current snapshot — *zero* new gaps) + reranked
  tally (GAP 3 → target 2/1/0).
- **Battery:** closed-class/determiner + the differential packing oracle (`packed ≡ unpacked`) stay green —
  the soundness gate for Lever 1.
- **Soundness:** no reading lost that a slower/unbounded parse would find — Lever 1 preserves it by
  construction (packing is exact on index-independent cells); Lever 2(a)/(b) must be witnessed as
  meaning-preserving (a lexicon unit ≡ its compositional reading; a left-branching tree ≡ the alternatives
  for these compounds).

## 6. Non-goals / risk log

- **Not PP-attachment control** — refuted (shelved note); the PPs parse.
- **Not the §3.3 adjective NF** — no-op for this corpus (gradable adjectives).
- **Realistic fallback:** these are 16-token sentences stacking 2–3 domain compounds; grammar is complete
  and 59/62 parse. If Levers 1–3 prove disproportionate, **accepting the 3-gap search-limited tail is a
  legitimate stopping point** — record it, don't grind. The value of this plan is bounded by whether Step 0
  shows a clean dominant lever.
