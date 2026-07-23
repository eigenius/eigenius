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

## Takeaway

Single-skeleton is a WEAK correctness signal — 3 of 4 checked were wrong. The faithfulness corpus grows
only on VERIFIED readings (now 20/20); these 3 units stay UNPINNED until fixed. Defect 1 (predicative
adjective + PP) is a clean grammar target; Defect 2 is two sense/glossary levers; Defect 3 needs a
structural read before it is a target.
