# D63 — single-skeleton wrong-reading defects (WRN first page)

Bucketing the 62 first-page units by structural-skeleton count (reranked, drift-free replay of
`2026-07-23-1533`): 21 units have a SINGLE skeleton. 17 were already faithfulness-pinned; the other 4
were checked here (the "easy pins" hypothesis). **Only 1 of the 4 was correct** — single-skeleton does
NOT imply correct, so the remaining 3 are wrong-single-reading defects, recorded below. Method: trace
with the recorded ranks + `EIGENIUS_GLOSS_READINGS=1`.

- ✅ **PINNED** — "We identified three groups of cell lines." A cardinal generalized-quantifier
  (`ΠG#0:Prop. ΠG#1:ΣG#1:§. prep_of(G#1, kind_of(§)). §(G#1.1, speaker) → G#0 → G#0`), verbalizer
  brackets Π-CPS by design; structure correct, cardinality "three" not encoded (same accepted caveat as
  "We analysed two … data sets"). Faithfulness 19 → 20.

## Defect 1 — predicative adjective + PP complement ("dependent on X")

"The lines from rare lineages were **less dependent on WRN**." → gloss *"the line from a rare line be a …
less **WRN protein**, human"*. `were less dependent on WRN` is mis-parsed: "dependent" (predicative
adjective) taking the PP complement "on WRN" is instead read as an ATTRIBUTIVE adjective on a head noun
"WRN" (→ "a … dependent WRN protein"), a predicate nominal. The comparative "less" and the copula "were"
survive, but the adjective's PP-complement frame (`dependent on _`) is not licensed. Likely a real
grammar gap (predicative-adjective subcategorized PP), not a sense/glossary issue — the WRN sense is even
correct (C1337007). 12 sense-variants, 1 (wrong) skeleton.

## Defect 2 — function-word "some" reified + "MSS" glossary miss

"**Some** MSI lines and **some MSS** lines were represented by these screening data sets." → gloss *"the
Disease Screening Data Set represent a **Some (qualifier value)** Microsatellite Instability line and …
a Some (qualifier value) **Marinesco-Sjogren syndrome** line"*. The verb structure is right (passive
`represent(data-set, lines)`, distributed over the coordination), but two argument defects:

- **"some" reification** — the determiner "some" seeds UMLS `C0205392` "Some (qualifier value)", piled
  into `compound_kind(MSI, C0205392)` — an EXTRA compound level, so the skeleton is structurally wrong,
  not just sense-wrong. Same family as the T078/T080 "and"/"For" reifications; the filter did not catch a
  qualifier-value determiner. Lever: extend the function-word / reification skip to determiner-colliding
  UMLS qualifier concepts.
- **"MSS" mis-grounded** — `C0024814` "Marinesco-Sjogren syndrome" (an abbreviation collision) instead of
  "microsatellite stable". "MSS" is not introduced with a parenthetical definition in the CNL, so the
  Schwartz-Hearst abbreviation glossary never binds it. Lever: a document glossary entry for MSS (a
  definitions-section / LLM abbreviation source, or the named-entity/acronym path).

## Defect 3 — coordinated predicate-nominal ("X are A, B and C")

"These groups are MSI lines, microsatellite-stable lines and indeterminate lines." → reading
`And(And(λG#0. the(group, MSI-line, G#0), λG#0. the(group, ms-stable-line, G#0)), λG#0. the(group,
indeterminate-line, G#0))` — a conjunction of three OPEN predicates (`λG#0. …`), which the verbalizer
cannot render (each conjunct bracketed). The predicative complement "A, B and C" builds a coordinated
predicate, but the subject "these groups" does not appear applied at the top — the reading looks like an
open predicate, not a closed proposition (yet the unit is bucketed parsed, not open). Needs structural
verification: is this the intended copula-predication of a coordinated NP complement, or is the subject
dropped? 10 sense-variants, 1 skeleton.

## Systematic analysis (confirmed by frame-probing)

### Defect 1 — root cause + scope (CONFIRMED broad)

Probes (cap-only, all structural readings):

- "WRN was essential." → `gt(essential(WRN), std)` — predicative gradable adjective works ALONE.
- "The gene was dependent on WRN." → `And(gt(dependent(gene), std), prep_on(gene, WRN))`.
- "WRN was essential for proliferation." → `And(gt(essential(WRN), std), prep_for(WRN, proliferation))`.
- "MSI is associated with responses." → `And(gt(assoc(MSI), std), prep_with(MSI, responses))`.

The PP is ALWAYS attached as a SEPARATE `And` conjunct `prep_X(subj, obj)` — "subj is Adj AND subj is
X-related-to obj" — never as the adjective's complement. **Root cause:** a WordNet adjective's sem is a
one-place gradable property `gt(deg_a(x), std_a)` with NO relatum slot, so "dependent"/"essential" cannot
consume "on WRN"/"for proliferation" as an argument; the copula's predicative-complement path conjoins
the adjective and the PP over the shared subject. The intended relational reading `dependent_on(gene,
WRN)` does not exist in the parse space. Recurs across: dependent-on / essential-for / associated-with /
dispensable-in / concordant-with — several of the ambiguity-tail units.

**Fix options:** (a) a rule that reinterprets `predicative-adjective + PP` as a two-place relation —
requires the adjective to expose a relatum, which the one-place WordNet sem does not, so this needs a
relational adjective encoding (deep); (b) accept the `And(gt(adj(subj)), prep_X(subj, obj))` conjunction
as the canonical CNL encoding, SUPPRESS the competing attributive reading (the "dependent WRN protein"
that beat it in unit 4), and let the And-reading pin. Difficulty: MEDIUM–HARD (a semantic-modeling
decision, not a local fix).

### Defect 2 — two independent sense/grounding levers (CONFIRMED)

- **2a "some" reification.** "Some cancers are common." → BOTH skeletons carry `compound_kind(cancer, §)`
  where `§` = `C0205392` "Some (qualifier value)": "some" reifies as a NOUN compounded onto the head, and
  the existential/determiner reading is ABSENT (no GQ/exists structure in any reading). Same family as the
  T078/T080 `and`/`For`/`each` reifications, but a qualifier-value colliding with a DETERMINER. **Fix:**
  importer-side skip of determiner-colliding qualifier concepts (extend the function-word filter) and/or a
  winning determiner entry for "some" (needs a reseed). Difficulty: MEDIUM.
- **2b "MSS" mis-grounded.** "MSS lines are common." is structurally fine (`subclass_of(MSS-line,
  common)`) but "MSS" grounds to `C0024814` "Marinesco-Sjogren syndrome" (an abbreviation collision), not
  "microsatellite stable" — MSS has no parenthetical definition in the CNL so the Schwartz-Hearst
  glossary never binds it. **Fix:** a document-glossary entry for MSS (definitions-section / LLM
  abbreviation source / acronym path). Difficulty: EASY–MEDIUM.

### Defect 3 — coordinated predicative complement (CONFIRMED structural)

"The groups are MSI lines and MSS lines." → `And(λG#0. the(class, MSI-line, G#0), λG#0. the(class,
MSS-line, G#0))` — the SAME open-λ coordinated predicate as with "These" (so it is NOT the demonstrative
going anaphoric / D64-open; it is the COORDINATION). Single-predicate copula applies the subject (the
pinned "These MSI cell lines were distinct" is closed); coordinating the predicative complement ("are A
and B") instead yields an `And` of open predicates with the subject apparently unapplied. **Needs a
code-level read** of the copula + predicative-complement coordination to confirm whether the resulting
sem is a closed `Prop` (subject consumed elsewhere) or a genuinely open predicate (bug). Difficulty:
MEDIUM (structural; investigate before committing to a fix).

## Takeaway

Single-skeleton is a WEAK correctness signal — 3 of 4 checked were wrong. The faithfulness corpus grows
only on VERIFIED readings (now 20/20); these 3 units stay UNPINNED until fixed. Priority read:
**Defect 1** is the broadest (correctness + a multiplicity lever across the tail) but a semantic-modeling
decision; **Defect 2** is two clean sense/glossary levers; **Defect 3** needs a code read first.
