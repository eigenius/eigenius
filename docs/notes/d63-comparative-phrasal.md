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

### 5.3 Emission + detection (the importer's job, for #8 only)

The count path (#9) is pure grammar; the **degree** path needs a lexical **gradable class**, emitted by
the importer:

- **Gradable adjective** → a `cat_measure` (`deg_A : Entity → float`) reading, in addition to its positive,
  so `more`/`less`/`-er` can operate. *Detection:* WordNet marks adjective gradability — antonym/gradable
  clusters + the **`attribute`** relation (`heavy`/`light` ↔ `weight`).
- **Relational adjective** → `deg_A : Entity → Entity → float` + the subcategorized preposition
  (`dependent on`, `sensitive to`). *Detection:* the adjective's typical PP complement.
- **Nominalization** → project the adjective's `deg_A` onto the noun as its `μ` (a `cat_measure` reading of
  `dependence`, alongside its plain `cat_n`). *Detection:* WordNet **derivational links** (`dependent →
  dependence`).
- **Operators** (`more/less/greater/fewer/than`) → closed-class (bootstrap), **not** importer.

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

## [ ] Phase A — Demo mechanism (no reseed; experiments/lexicon/lexicon.esl + kernel/tests)
Prove the whole grammar before any bootstrap/importer commitment.

A1 — Fix the mislabeled fewer (§5.8). Rename demo fewer_cmp → less (its gt(μ(y),μ(x)) over cat_measure is correct for less); flip the phrasal test so *fewer dependence gets no parse. Prereq for A2; corrects the committed error.
A2 — #9 cardinality operator (§5.1). Add fewer/more(count) over cat_n: (((S\NP)/cat_pp_than)\((S\NP)/NP))/cat_n(T,num), sem λN.λV.λy.λx. gt(card(N,y),card(N,x)); add the opaque card : Set→Entity→float axiom. Test the cardinality denotation + the compound (deletion mutations via KindCompound) + restrictive PP (in …) composing before the count.
A3 — #8 adjectival more/less (§5.2, §5.5(b) surgical). Give a demo gradable adjective a deg_A (cat_measure) reading; add more/less in the predicative frame ((S[adj]\NP)/cat_pp_than)/cat_measure, sem λμ.λy.λx. gt(μ(x),μ(y)). Add a relational dependent on X (deg_dependent, cat_measure/cat_pp_arg). Test X is more/less dependent on Y than Z.
A4 — Unify the noun & adjective scale (§5.0/§5.2). Make the demo noun dependence's μ = adjective dependent's deg_dependent (one axiom); assert more dependent on WRN and greater dependence on WRN produce the identical denotation.

## [ ] Phase B — Promote operators to closed-class + reseed → closes #9 at scale
B1 — Move operators to closed-class.esl (bootstrap). greater/less/more/fewer + the card functor + scale plumbing. (than/cat_pp_than is already closed-class.) Measure nouns / gradable adjectives stay out (demo/importer).
B2 — Reseed + verify #9 at scale. #9 operates over existing cat_n, so it closes with just the operators — no importer emission needed. Probe cells contained fewer mutations than genes: GAP→CLOSED. #8 still GAPs.

## [ ] Phase C — #8 degree at scale: importer gradable emission (the design effort; §5.3)
Prereq — grounding pass: confirm the WordNet detection signals (gradable/antonym clusters, attribute relation, derivational-link coverage) are good enough before building. Also verify the §6 anchor DOIs.

C1 — Gradable-adjective detection + emission → a deg_A (cat_measure) reading per detected adjective.
C2 — Relational-adjective emission → deg_A : Entity→Entity→float + the governed preposition.
C3 — Nominalization projection → the noun's cat_measure reading, μ = the adjective's deg_A, via derivational links.
C4 — Reseed + verify #8 at scale, managing the ambiguity/beam cost (§5.6: genuinely-gradable only, low rank, beam).

## [ ] Phase D — Shared NP-complexity gaps (§5.7; independent, interleavable)
Needed for the full corpus sentences, independent of the comparative: complex subject (MSI cell lines from these four lineages — modifier + plural compound + these four demonstrative+cardinal) and possessive than-object (their MSS counterparts). Separate gaps (determiner+number, possessive), tracked in d63-parse-gap-closure.md.

## [ ] Phase E — Deferred: Kennedy factoring (§5.5(a))
Refactor the predicative slice to deg_A + pos/cmp/sup operators (the eventual structure). Not needed to close #8/#9.

Corpus-sentence closure: #9's full sentence closes after B2 + D; #8's after C4 + D.

