# WRN-helicase — DCG parser test corpus (litmus)

**Purpose.** A coverage litmus for the D63 DCG engine: the kind of real scientific prose the parser
must eventually process, paired with the canonical propositions it should yield. Each entry is a
*parse gate* (does it parse at all?) and a *faithfulness gate* (does it parse to the **right**
proposition? — the D61 back-translation gold).

**Provenance / status (read before trusting).**
- The **propositions** are *authoritative* — the `reasoning:proposition` / `reflection:canonical_proposition`
  values committed in `chain/04`–`chain/09` (kernel-checked).
- The **English** is a **Declared gloss** (rendered from the predicate names + the `onco` ontology, not the
  source paper). To curate against the actual paper prose (Chan et al.) is a follow-up — these are a
  *tier-1 clean-prose* target, not the messier source text.
- **Not yet runnable.** None of these parse today: they need (a) the unbuilt nominal/adjunct constructions
  below, and (b) a **domain lexicon** (`MSI`, `WRN`, `depletion`, …; not in WordNet — D62 §8.7.8). The
  executable parse/no-parse coverage harness lands once a domain lexicon exists; until then this file is the
  static gold + the priority signal.
- **Compositional ≠ opaque predicate.** Even with full grammar, "WRN is selectively essential in MSI" won't
  *compositionally* yield `SelectivelyEssential(WRN, MSI)` — that mapping is the D62 encoding institution's
  job (LLM proposes the domain predicate, kernel felicity-gates). The grammar delivers the felicitous
  skeleton; the institution maps it to the predicate.

## Coverage summary — blocked-on (the build-priority signal)

Counts of corpus sentences blocked on each **unbuilt** construction (a sentence can be blocked on several).
Built constructions (copula, transitive, predicate-nominal, determiners, agreement, passive, coordination,
negation, relatives, complements, modals, comparatives) are *not* listed — they're ✅.

| Construction (unbuilt) | ~sentences blocked | D63 status |
|---|---|---|
| **compound noun** (N-N: "WRN depletion", "mutator load") | ~26 | not built — **top priority** |
| **PP adjunct** (NP/VP modifier: "in MSI", "of WRN dependency", "for the dependence") | ~19 | not built — **high** |
| **domain lexicon** (MSI/WRN/TP53/… entries) | all | needed (D62 §8.7.8) — not grammar |
| **hyphenated compound** ("MSI-selective", "DSB-driven", "wild-type") | ~9 | not built |
| **adverb** ("selectively", "partially") | ~4 | deferred (§8.7.5) |
| **conditional sentence** ("if … then …") | ~3 | not built |
| **possessive** ("WRN's") | ~3 | deferred (6-tail) |
| **gerund-clause subject** ("Restoring MMR …") | ~2 | deferred (6-tail) |
| **control / to-infinitive** ("fails to rescue") | ~1 | deferred (frames 24/25) |
| **fronting / focus** ("Among …, only WRN …") | ~1 | deferred |

→ **compound nouns + PP adjuncts dominate** (in nearly every sentence). They are the highest-leverage next
D63 slice for this corpus — **designed as D63 §8.13 (Slice 6-mod, nominal modification)**: opaque
modification reusing 3b's Σ-refine + a PP VP-adjunct, with the precise relation grounding-supplied by the
D62 institution.

## The corpus

Legend (blocking column lists the *unbuilt* constructions each needs): `cmpN` compound noun ·
`PP` prepositional adjunct · `adv` adverb · `poss` possessive · `ger` gerund subject · `cond` conditional ·
`hyph` hyphenated compound · `ctrl` control/to-infinitive · `focus` fronting/focus · `lex` domain lexicon.

### Phase 1 — recompute conclusions (`chain/04`)
| # | English (Declared gloss) | Canonical proposition | Blocking |
|---|---|---|---|
| 1 | WRN is selectively essential in MSI cancers. | `SelectivelyEssential(WRN, MSI)` | adv, PP, cmpN, lex |
| 2 | WRN dependency correlates with mutator load in MSI. | `DependencyCorrelatesWithMutatorLoad(WRN, MSI)` | cmpN, PP, lex |
| 3 | Mutator load is elevated in the common MSI lineages relative to the uncommon ones. | `ElevatedMutatorLoadInCommonLineages(MSI_common, MSI_uncommon)` | cmpN, PP, lex |
| 4 | WRN shows reduced nucleolar colocalization in MSI. | `ReducedNucleolarColocalization(WRN, MSI)` | cmpN, PP, lex |
| 5 | WRN depletion causes apoptosis in MSI. | `CausesApoptosis(WRN, MSI)` | cmpN, PP, lex |
| 6 | WRN depletion causes cell-cycle arrest in MSI. | `CausesCellCycleArrest(WRN, MSI)` | cmpN, hyph, PP, lex |
| 7 | MSI cells selectively depend on WRN for viability. | `SelectiveViabilityDependence(WRN, MSI)` | cmpN, adv, PP, lex |
| 8 | Restoring MMR in HCT116 restores mismatch repair. | `MMRRestorationRestoresRepair(HCT116, Ch3plus5)` | ger, PP, cmpN, lex |
| 9 | Among the RecQ helicases, only WRN is MSI-selective. | `OnlyMSISelectiveInFamily(WRN, RecQ_helicases)` | focus, hyph, cmpN, lex |
| 10 | MSI is a strong biomarker of WRN dependency. | `StrongBiomarker(MSI, WRN_dependency)` | PP, cmpN, lex |
| 11 | TP53 status modulates the WRN dependence. | `ModulatesDependence(TP53, WRN)` | cmpN, lex |
| 12 | MMR restoration partially rescues the WRN dependence. | `RestorationPartiallyRescues(dMMR, WRN)` | cmpN, adv, lex |
| 13 | Wild-type WRN cDNA rescues sgWRN depletion. | `RescuesDepletion(WRN_cDNA_WT, sgWRN_EIJ)` | hyph, cmpN, lex |

### Phase 1 — discovery (`chain/05`)
| # | English (Declared gloss) | Canonical proposition | Blocking |
|---|---|---|---|
| 14 | WRN is the top differential dependency in MSI in the Achilles screen. | `TopDifferentialDependency(WRN, Achilles_MSI)` | cmpN, PP, lex |
| 15 | For any gene that is a top differential dependency in MSI in both the Achilles and DRIVE screens, that gene is selectively essential in MSI. | `∀g. TopDifferentialDependency(g, Achilles_MSI) → TopDifferentialDependency(g, DRIVE_MSI) → SelectivelyEssential(g, MSI)` | cond, cmpN, PP, lex |

### Phase 2 — validation (`chain/07`)
| # | English (Declared gloss) | Canonical proposition | Blocking |
|---|---|---|---|
| 16 | WRN's helicase activity is required for the dependence. | `RequiresActivity(WRN, helicase)` | poss, cmpN, PP, lex |
| 17 | WRN's exonuclease activity is dispensable for the phenotype. | `DispensableActivity(WRN, exonuclease)` | poss, cmpN, PP, lex |
| 18 | WRN's helicase and exonuclease activities are separable. | `litclaim:WRNActivitiesSeparable(WRN)` | poss, cmpN, lex |
| 19 | The K577M helicase-dead construct fails to rescue the depletion. | `FailsToRescue(WRN_cDNA_K577M, sgWRN_EIJ)` | ctrl, hyph, cmpN, lex |
| 20 | The WRN MSI-viability effect is on-target. | `OnTarget(WRN, MSI_viability)` | cmpN, hyph, lex |

### Phase 3 — in-vivo & mechanism (`chain/08`)
| # | English (Declared gloss) | Canonical proposition | Blocking |
|---|---|---|---|
| 21 | The WRN dependence holds in vivo in MSI. | `InVivoDependence(WRN, MSI)` | cmpN, PP, lex |
| 22 | WRN depletion causes DNA double-strand breaks in MSI. | `CausesDSBs(WRN, MSI)` | cmpN, hyph, PP, lex |
| 23 | WRN depletion activates the DSB response in MSI. | `ActivatesDSBResponse(WRN, MSI)` | cmpN, PP, lex |
| 24 | The WRN lethality in MSI is DSB-driven. | `DSBDrivenLethality(WRN, MSI)` | cmpN, PP, hyph, lex |
| 25 | WRN depletion activates the p53 response in MSI. | `ActivatesP53Response(WRN, MSI)` | cmpN, PP, lex |
| 26 | The WRN dependence in MSI is not via a telomere defect. | `NotViaTelomereDefect(WRN, MSI)` | PP, cmpN, lex |
| 27 | The WRN dependence in MSI is not explained by paralog loss. | `NotExplainedByParalogLoss(WRN, MSI)` | PP, cmpN, lex (passive+neg ✅) |
| 28 | The C911 control is a valid on-target control for shRNA. | `litclaim:C911ControlIsValid(shRNA)` | cmpN, hyph, PP, lex |
| 29 | p53-S15 phosphorylation marks p53 activation. | `litclaim:pS15MarksP53Activation(p53)` | cmpN, lex |

### Phase 5 — synthesis (`chain/09`)
| # | English (Declared gloss) | Canonical proposition | Blocking |
|---|---|---|---|
| 30 | If restoring MMR partially rescues the WRN dependence, then the MMR defect contributes to it. | `RestorationPartiallyRescues(dMMR, WRN) → ContributesToDependence(dMMR, WRN)` | cond, ger, cmpN, lex |
| 31 | If WRN is selectively viability-dependent and in-vivo dependent in MSI, requires helicase activity, is DSB-driven-lethal, the MMR defect contributes, and it is not explained by paralog loss, then WRN is synthetic-lethal with the MSI state. | `SelectiveViabilityDependence → InVivoDependence → RequiresActivity(helicase) → DSBDrivenLethality → ContributesToDependence(dMMR) → NotExplainedByParalogLoss → SyntheticLethal(WRN, MSI)` | cond, hyph, cmpN, lex |

### Quantitative evidence sub-statements (`chain/03` recompute certificates)
These are the `stats:` propositions the conclusions' certificates discharge — measured facts, not surface
prose. Listed for completeness; they are produced by the kernel/recompute, not by parsing English.
| English (gloss) | Proposition |
|---|---|
| The MSI-vs-MSS mean dependency difference is below zero (a selective depletion effect). | `stats:lt(stats:mean_diff_of(S), 0.0)` — 14 sample sets |
| Dependency vs. mutator-load Spearman ρ is below zero. | `stats:lt(stats:spearman_rho(wrn_corr_sampleset), 0.0)` |
| The MSI→WRN-dependency biomarker PPV is ≥ 0.70. | `stats:ge(stats:ppv(wrn_dep_sampleset), 0.7)` |
