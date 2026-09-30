# From opaque predicates to propositions — where the paper states each result

The chain states the paper's results with 31 `onco:` and 8 `litclaim:` predicates
([`01-onco.esl`](../chain/01-onco.esl), [`02-literature.esl`](../chain/02-literature.esl)). Each is
declared `data onco:X : core:string -> core:string -> Prop { }` — no constructors, applied to
strings (`onco:SelectiveViabilityDependence("WRN", "MSI")`). A conclusion's grounds are propositions
over the data (`stats:lt(stats:mean_diff_of(<sample set>), 0.0)`); a declared bridge
(`bridge_viability : K -> O -> SelectiveViabilityDependence("WRN", "MSI")`) lifts them into the
opaque predicate. The kernel checks the lift, and nothing about what the predicate says.

Replacing a predicate with a proposition in the kernel's type theory takes two things from the paper:

1. **The sentence that states the result**, parsed into a proposition over lexicon concepts — `WRN`
   the gene, MSI cell lines a kind — instead of a name over strings.
2. **The text that defines its terms operationally**, which the bridge becomes: a definition
   connecting the proposition to the statistic the conclusion rests on, where today it is a declared
   implication into a name.

This document locates both for every predicate.

## The source

The PubMed Central author manuscript, **PMC6580861** (NIHMS1522798) — the version this case study's
line references already cite. Its JATS carries the abstract, the Letter's body, the methods, all 14
figure legends and the data-availability statement as separate elements.

- `data/fetch.sh` fetches it (`pmc_jats`, NCBI efetch) and derives the text (`pmc_text`,
  [`extract/jats_to_text.py`](../extract/jats_to_text.py)); both are pinned by sha256 in
  [`data/sources.tsv`](../data/sources.tsv).
- The text is one paragraph per line, each with a locator: `[abstract.p0]`, `[body.pN]`,
  `[methods/<subsection>.pN]`, `[figN pK]` and `[ed-figN pK]` (p0 the legend's title, p1 its panels),
  `[body/code-data-availability.pN]`. A Letter's body has no headings: results and discussion are
  `body.p0`–`body.p15`.
- The OCR texts under `references/publications/WRN-Helicase-Nature-OCR/` are Nature's version of
  record, which differs in places (`931g` / `931 RCF`, D96), and they omit the legends.

## The predicates a conclusion uses (27)

"Stated at" is the sentence to parse; "defined at" is the text the bridge should become. Legends
supply each panel's n, test and error bars alongside the methods.

| Predicate | Stated at | Defined at | Conclusions |
|---|---|---|---|
| `TopDifferentialDependency(WRN, screen)` | `body.p2` "each independently identified WRN … as the top preferential dependency in MSI compared to MSS cell lines" | `methods/differential-dependency-analysis` (limma, difference in mean dependency, moderated t, Benjamini–Hochberg Q); `fig1 p1` | 1 |
| `SelectivelyEssential(WRN, MSI)` | `abstract.p0` "WRN was selectively essential in MSI models in vitro and in vivo, yet dispensable in microsatellite stable (MSS) models"; `ed-fig1 p0` | as above; `methods/genetic-dependency-data` | 2 |
| `OnlyMSISelectiveInFamily(WRN, RecQ)` | `body.p2` "none of the four other RecQ DNA helicases were preferentially essential with MSI" | `methods/differential-dependency-analysis`; `ed-fig1 p1` (c) | 1 |
| `StrongBiomarker(MSI, WRN)` | `body.p2` "the MSI/WRN relationship compared favorably to other strong biomarkers for vulnerabilities" | `methods/dependency-and-biomarker-analysis` (dependent: average CRISPR/RNAi score < −0.5; PPV, sensitivity) | 1 |
| `ElevatedMutatorLoadInCommonLineages(MSI_common, MSI_uncommon)` | `body.p3` "harboring a median 0.56-fold fewer deletion mutations in microsatellite regions compared to typical-lineage MSI models"; `ed-fig2 p0` | `methods/microsatellite-classification` (normalised MS deletions); `ed-fig2 p1` | 1 |
| `DependencyCorrelatesWithMutatorLoad(WRN, MSI)` | `body.p3` "WRN dependency correlated with the number of microsatellite deletions within all MSI cell lines and in MSI-predominant lineages" | `methods/microsatellite-classification`; `ed-fig2 p1` (Spearman) | 1 |
| `SelectiveViabilityDependence(WRN, MSI)` | `body.p4` "WRN depletion impaired the viability of MSI cells despite negligible effects in MSS cells"; "impaired MSI, but not MSS, cell viability"; `ed-fig3 p0` | `methods/cell-viability-assay`, `methods/luciferase-competitive-growth-assay`; `fig2 p1`, `ed-fig3 p1` (two-way ANOVA) | 2 |
| `RescuesDepletion(construct, reagent)` | `body.p4` "WRN cDNA rescued Cas9-expressing KM12 from sgWRN-EIJ, but not sgWRN2"; `body.p5` "Exonuclease inactivation did not attenuate rescue" | `methods/generation-of-ectopic-wrn-cdna-expressing-cell-l`, `methods/cell-viability-assay`; `fig2 p1` (b, c) | 4 |
| `FailsToRescue(construct, reagent)` | `body.p5` "helicase inactivation prevented rescue" | as above | 2 |
| `OnTarget(WRN, MSI_viability)`, `(WRN, xenograft_growth)` | `body.p4` "the viability loss in MSI cells is attributable to WRN inactivation"; `body.p6` (the C911 control, in vivo) | the rescue design, `body.p4`; `methods/shrnas` | 2 |
| `RequiresActivity(WRN, helicase)` | `abstract.p0` "MSI cancer models required the helicase activity, but not the exonuclease activity of WRN"; `body.p5` | the rescue logic, `body.p5`; `fig2 p1` | 2 |
| `DispensableActivity(WRN, exonuclease)` | `body.p5` "suggesting this function is dispensable" | as above | 1 |
| `InVivoDependence(WRN, MSI)` | `body.p6` "Induction of shWRN1 but not shWRN1-C911 significantly impaired tumor growth"; "impaired the viability of a novel patient-derived organoid" | `methods/in-vivo-xenograft-studies`; `fig2 p1` (d–g) | 2 |
| `SeedControlInert(WRN, xenograft_growth)` | `body.p6` "control (shWRN1-C911) with nucleotides 9 through 11 of shWRN1 mutated to its complement, thus maintaining the 'seed' sequence" | `methods/shrnas`; ref. 16 | 1 |
| `CausesCellCycleArrest(WRN, MSI)` | `body.p7` "WRN silencing reduced the proportion of MSI cells in S phase and increased cells in G1 or G2/M phases, suggesting cell cycle arrest"; `abstract.p0`; `ed-fig4 p0` | `methods/cell-cycle-analysis`; `ed-fig4 p1` | 3 |
| `CausesApoptosis(WRN, MSI)` | `body.p7` "demonstrated induction of apoptosis and cell death in MSI cells following WRN silencing"; `abstract.p0` | `methods/apoptosis-assay`; `ed-fig4 p1` | 4 |
| `ModulatesDependence(TP53, WRN)` | `body.p8` "p53 activity contributes to, but is not solely responsible for WRN dependence"; "p53-intact MSI cell lines … were more sensitive to WRN loss than their p53-impaired … counterparts" | `methods/cell-line-annotations` (TP53 status); `ed-fig5 p1` (g) | 1 |
| `CausesDSBs(WRN, MSI)` | `abstract.p0` "WRN depletion induced double-strand DNA breaks"; `body.p9` "WRN silencing in MSI but not MSS cells substantially increased ɣH2AX and 53BP1 foci, markers of DSB"; `fig4 p0`, `ed-fig6 p0` | `methods/immunofluorescence` (foci, intensity); `fig4 p1`, `ed-fig6 p1` | 6 |
| `ActivatesDSBResponse(WRN, MSI)` | `body.p9` "corroborated by increased phospho-ATM (S1981) foci formation and Chk2 (T68) phosphorylation, indicating DSB responses"; `ed-fig7 p0` | `methods/immunofluorescence`, `methods/immunoblotting`; `ed-fig7 p1` | 1 |
| `DSBDrivenLethality(WRN, MSI)` | `body.p10` "DSBs precipitate the lethal effects of WRN loss and are not merely a consequence of cell death"; `body.p15` | `methods/telomere-pna-fish-of-metaphase-spreads`; `fig4 p1` (d, e) | 2 |
| `NotViaTelomereDefect(WRN, MSI)` | `body.p10` "we did not observe specific telomeric defects such as increased chromosomal end-to-end fusions or telomeric signal loss" | `methods/telomere-pna-fish-of-metaphase-spreads`; `ed-fig8 p1` (a) | 1 |
| `ReducedNucleolarColocalization(WRN, MSI)` | `body.p11` "predominantly dispersed staining across the nucleoplasm in MSI cells but greater WRN co-localization with the nucleolar marker fibrillarin … in MSS cells"; `ed-fig8 p0` | `methods/immunofluorescence`; `ed-fig8 p1` (d, two-tailed t) | 1 |
| `NotExplainedByParalogLoss(WRN, MSI)` | `body.p12` "we found no gene whose loss could account for the preferential dependency upon WRN with MSI"; `ed-fig9 p0` | `methods/assessing-potential-wrn-synthetic-lethality` (per-gene linear model); `ed-fig9 p1` | 2 |
| `MMRRestorationRestoresRepair(HCT116, Ch3plus5)` | `body.p13` "MMR activity of the … HCT116 was restored by introducing chromosomes 3 and 5" | `methods/fluorescence-based-multiplexed-host-cell-reactiv`; `ed-fig10 p1` (a) | 1 |
| `RestorationPartiallyRescues(dMMR, WRN)` | `body.p13` "Ch3+5 transfer suppressed DSB accumulation and partially rescued viability from shWRN" | `methods/cell-viability-assay`, `methods/clonogenic-assay`; `fig4 p1` (f), `ed-fig10 p1` | 3 |
| `ContributesToDependence(dMMR, WRN)` | `body.p14` "dMMR alone contributes to but does not fully explain this synthetic lethal interaction"; `ed-fig10 p0` | `body.p13` (the restoration and MLH1-knockout logic) | 2 |
| `SyntheticLethal(WRN, MSI)` | `abstract.p0` "expose WRN as a synthetic lethal vulnerability and promising drug target for MSI cancers"; `body.p15`; `fig2 p0` "WRN is a synthetic lethal partner with MSI." | **the paper defines the term**: `abstract.p0` "Synthetic lethality, an interaction whereby the co-occurrence of two genetic events leads to cell death but one event alone does not" | 1 |

`litclaim:WRNActivitiesSeparable` is the one literature predicate a conclusion uses: `body.p5` "WRN
protein functions as both a 3'−5' exonuclease and 3'−5' helicase … [14],[15]", and the mutant
versions "of WRN cDNA [14]".

## The predicates no conclusion uses (11)

- **`MSI`, `MSS`, `MMRloss`** are classes the paper defines in the methods: MSI and MSS by
  `methods/microsatellite-classification` (deletion counts and the fraction in microsatellite
  regions, from CCLE Phase II), MMR loss by `methods/mmr-status` (MSH2, MSH6, MLH1 or PMS2 mutated,
  deleted or lowly expressed). They are definitions over the cell-line annotations, not results.
- **`ViabilityDependenceAtBiologicalUnit`** is this encoding's re-analysis (recompute finding F4), not
  a claim the paper makes; it stays an encoding-side predicate.
- **Seven `litclaim:` predicates** are authored for provenance only. Each is a citing sentence:
  `SyntheticLethalityExploitable` `abstract.p0` [1]; `WRNNucleolarDynamics` `body.p11` [20];
  `CERESCorrectsCopyNumber` and `DRIVEDependencyDataset` `body.p1` [10],[11] and
  `methods/genetic-dependency-data`; `LimmaModeratedT` `methods/differential-dependency-analysis`
  [36]; `VoomPrecisionWeights` and `HallmarkGeneSets` `methods/differential-expression-analysis`
  [40], [43].

## What each part of the paper carries

| Part | Carries | Locators |
|---|---|---|
| Abstract | the thesis, the headline findings, and the definition of synthetic lethality | `abstract.p0` |
| Body | every finding's stating sentence, and the discussion's synthesis and hedging | `body.p1`–`body.p15` |
| Legend titles | 14 one-sentence findings (`WRN depletion preferentially impairs MSI cell viability.`) | `figN p0`, `ed-figN p0` |
| Legend panels | per panel: what was measured, n, the test, the error bars | `figN p1`, `ed-figN p1` |
| Methods, data and analysis | what dependency, MSI, MMR loss, TP53 status, preferential dependency, the paralog test and the expression analysis are | 10 subsections |
| Methods, reagents and assays | what each readout measures (viability, competitive growth, clonogenic, cell cycle, apoptosis, IF, immunoblot, xenograft, FISH, HCR) | 15 subsections |
| Data availability | the source data and datasets the Observed nodes pin | `body/code-data-availability` |
| References | the sources of the `litclaim:` predicates | not in the text; the JATS `ref-list` |

## What parsing the stating sentences needs

The stating sentences are journal prose; each needs a controlled-English rendering that keeps its
claim (`docs/method/controlled-english-style-guide.md`, R1 and R2). The constructions they rely on,
and where each stands:

- **Comparatives with a standard**: `showed greater WRN dependence than their MSS counterparts`,
  `more sensitive to WRN loss than their p53-impaired counterparts`, and with a measured difference,
  `0.56-fold fewer deletion mutations`. The degree and differential comparatives are D95 slice 7.
- **Governed prepositions**: `attributable to`, `account for`, `rescued KM12 from sgWRN-EIJ`,
  `dependence on`, `correlated with`. D97 supplies the governance; its decision 6 is whether the
  preposition reaches the meaning.
- **Negation and absence**: `did not observe specific telomeric defects`, `no gene whose loss could
  account for`, `none of the four other RecQ DNA helicases`.
- **Contrast**: `in MSI but not MSS cells`, `required the helicase activity, but not the exonuclease
  activity` — same-kind `but not`, which the style guide lists as covered.
- **Report and hedge verbs**: `suggest`, `argue`, `indicating`, `nominating` — clausal complements.
- **Causal verbs**: `induce`, `promote`, `lead to`, `precipitate`.
- **Quantities**: `96 hours after shWRN1 induction`, `8 days following sgRNA transduction` (D95). The
  hyphenated prenominal measure — `an 8-day viability assay`, `a 10-day competitive growth assay`,
  `a 7-day viability assay` — is read as one since D95 slice 7d (decision 12): a numeral joined by a
  hyphen to a unit name is a quantity token. `5-fold` stays a word; `fold` is not a unit.
- **Statistics** (`Q values = 4.8×10−24`, `P = 4.2×10−13`, `rho = −0.74`, `n = 37`) stay out of the
  claim and route to D52 records, per the style guide; the conclusions already carry them as
  `stats:` results.
