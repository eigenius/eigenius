# Encoding Plan — *WRN Helicase is a Synthetic Lethal Target in Microsatellite Unstable Cancers*

> **Stage-1 deliverable.** A detailed implementation + content plan for converting Chan et al., *Nature* 2019 (568:551–556, doi:10.1038/s41586-019-1102-x) from prose into a structured, machine-checkable Eigenius representation. Stage 2 (the actual encoding) is informed by this plan. Source text: [`../WRN-Helicase-nihms-1522798.txt`](../../../../references/publications/WRN-Helicase-nihms-1522798.txt).
>
> *Created 2026-06-12. Working document.*
>
> **Status: realized in full as of 2026-06-13.** This plan was executed end to end; the whole `H1 → … → C-MAIN` argument graph type-checks across Phases 1–5. The §8 phasing section is annotated in place with the dated as-built increment logs, and the [review memo](04-review.md) is the retrospective narrative. This document now doubles as plan-of-record and as-built log.

---

## 0. Purpose, scope, and strategy

The goal is to represent the paper's **epistemic structure** — every load-bearing element of *declared*, *observed*, *derived*, and (where applicable) *verified* information, plus the *logical dependencies* among them — so that the central claim (*WRN helicase is a synthetic-lethal vulnerability in MSI cancers, via its helicase activity*) is traceable, step by step, back to data, methods, and cited authority, and so that the data-backed steps can be **recomputed** by Eigenius institutions and checked against the paper's reported numbers.

**Why this paper.** It is the inverse of the ScienceAgentBench tasks (spec-adherence with hidden gold). It is real open-ended scientific reasoning: a hypothesis, a discovery from public data, layered validation, a mechanistic model, and a causal dissection — exactly the shape Eigenius is built to make auditable. It also has the property that the headline computational claims are **recomputable from public DepMap data**, so condition-C encoding can turn author-asserted *Derived* claims into institution-*recomputed* warrants that close the audit chain back to the raw screen data.

**Scope discipline (this is large).** A full encoding is dozens of experiments across ~20 figure panels and ~10 extended-data panels. Do **not** attempt it monolithically. Encode **spine-first, in phases** (§8); Phase 1 (the computational discovery) is self-contained, data-backed, recomputable, and on its own demonstrates the thesis. Later phases add the wet-lab validation and mechanism as primarily *provenance* (Observed → Derived with linked external executions), not recomputation.

**The four warrants in this paper (preview):**
- **Observed** — public screen/omics datasets; raw wet-lab measurements (viability, immunoblot, IF intensities/foci, tumor volumes, FISH, flow cytometry, mRNA-seq reads).
- **Derived** — statistical analyses (differential dependency, correlations, moderated *t*, BH-FDR, GSEA, mixed models, ANOVA) and upstream algorithmic pipelines (CERES, DEMETER2). Split into *institution-recomputable* vs *linked-external-execution* (§5).
- **Declared** — the hypothesis; the operational definitions/thresholds (MSI status, MMR loss, "dependent", TP53 status, POLE status); experimental design decisions (controls, the C911 seed control, the sgWRN-EIJ on-target design, exonuclease/helicase-dead constructs); and domain rules imported from the literature (§3).
- **Verified** — no *formal* (Lean-style) proofs exist in the biology. The strongest checkable tier is the **institution-recomputed Derived** claims (§2.4). One genuinely formal opportunity: encode the *synthetic-lethality logical schema* itself as a checkable proposition (§2.4).

---

## 1. The argument spine (what we are encoding)

The paper's reasoning, as a dependency graph (node IDs used throughout this plan; full edge list in §7):

```
H1  hypothesis: dMMR/MSI creates vulnerabilities
      │
      ▼  (query two independent dependency datasets)
D-DIFF  differential dependency (MSI vs MSS), Limma + BH, on Achilles & DRIVE
      │  ⇒ WRN is the TOP preferential dependency in BOTH (Q=4.8e-24 / 1.5e-45)
      ▼
C-WRN   WRN selectively essential in MSI (independent CRISPR + RNAi replication)
      ├─ D-RECQ   among 5 RecQ helicases only WRN is MSI-selective
      ├─ D-BIOM   MSI is a strong biomarker for WRN dep (PPV/sens ≈ KRAS/BRAF)
      └─ D-REFINE WRN dep needs MSI-predominant lineage + mutator load
                   (Spearman WRN-dep ~ #MS-deletions; rho=-0.74 / -0.57)
      │
      ▼  (wet-lab validation of the computational call)
C-VAL   WRN KO/KD impairs MSI but not MSS viability (multi-assay, ANOVA)
      ├─ D-ONTARGET  WRN-cDNA rescue + sgWRN-EIJ ⇒ effect is on-target
      └─ D-HELICASE  helicase-dead (K577M) fails to rescue; exonuclease-dead (E84A) rescues
                      ⇒ helicase activity required, exonuclease dispensable
      │
      ▼  (in vivo + patient model)
C-VIVO  shWRN1 (not C911 seed-control) impairs KM12 xenograft growth (lme4 LRT)
        + shWRN impairs MSI patient-derived organoid (CCLF_CORE_0001_T)
      │
      ▼  (mechanism)
C-MECH  WRN loss ⇒ DSBs ⇒ p53 activation, apoptosis, cell-cycle arrest — MSI-selective
        evidence: GSEA (G2/M & E2F down, apoptosis up, p53 up); p53-S15 & p21 IF;
        γH2AX/53BP1/pATM(S1981)/Chk2(T68); FISH (diffuse DSBs, NOT telomeric)
      │
      ▼  (causal dissection of dMMR's role)
C-MMR   dMMR contributes but does not fully explain: HCT116 Ch3+5 MMR-restoration
        partially rescues; MLH1-KO re-sensitizes; FM-HCR confirms MMR activity
      │
      ▼
C-MAIN  WRN (helicase domain) is a synthetic-lethal target for MSI cancers
```

Each node is a candidate `reasoning:ReasoningSentence` whose `JustificationTerm` cites the Observed/Derived/Declared resources beneath it. `C-MAIN` is the paper's thesis; its justification is the composition of the whole graph.

---

## 2. Warrant inventory

### 2.1 Observed — datasets and raw measurements

**Public datasets (fetch; §4 has repositories):**

| ID | Observed resource | Provenance |
|---|---|---|
| O-ACHILLES | CRISPR/Cas9 dependency (18Q4 Avana, CERES-scored), 517 lines | DepMap/figshare [31], CERES [10] |
| O-DRIVE | RNAi dependency (Project DRIVE, DEMETER2-scored), 398 lines | figshare [33], DEMETER2 [32] |
| O-OMICS | DepMap 18Q4 omics: gene-level expression (TPM), relative copy number, mutation calls | depmap.org [12,34] |
| O-RPPA | CCLE RPPA protein abundance (incl. MSH2/MSH6) | CCLE [10,12,34] |
| O-MSI | CCLE Phase-II MSI feature data (#deletions, fraction-in-MS, per data source: CCLE WES/WGS/hybrid-capture, Sanger WES) | CCLE Phase II [12] |
| O-TP53ANN | nutlin-3 sensitivity (GDSC), CTD2 portal data (inputs to the TP53-status *Declared* rule) | GDSC, CTD2 [35] |
| O-MSEQ | mRNA-seq of WRN-KO vs control in SW48 & OVK18 (2 biological replicates) | GEO **GSE126464** |
| O-HALLMARK | MSigDB Hallmark gene-set collection (GSEA input) | MSigDB [43] |
| O-WRNFIG | "DepMap Datasets for WRN manuscript" (curated analysis inputs) | figshare doi:10.6084/m9.figshare.7712756.v1 |
| O-CODE | analysis code (defines the exact Derived pipelines) | github.com/cancerdatasci/WRN_manuscript |

**Wet-lab measurements (Observed; encoded as `bench:Measurement`/assay-result resources with replication metadata — see note).** Each carries its replication structure, which feeds the statistics institution's scope-of-inference check (biological vs technical replicates; cf. `stats-and-reasoning.json`):
- Viability assays (CellTiter-Glo): 8-day KO, 10-day competitive growth, clonogenic, organoid (CCLF_CORE_0001_T). *Triplicate / n=6 biological / 2 bio × 3 tech (organoid).*
- Immunoblots (WRN, γH2AX, pChk2, MLH1, MSH3, GAPDH, …). *2–3× replicates.*
- Immunofluorescence intensities/foci (p53-S15, p21, γH2AX, 53BP1, pATM-S1981; WRN/fibrillarin co-localization). *≥1000 cells/sample; per-cell counts.*
- Flow cytometry: cell-cycle (EdU/DAPI), apoptosis (AnnexinV/PI). *2 independent × triplicate.*
- Telomere PNA-FISH metaphase spreads (DSB/fragmentation classification). *30–60 metaphases.*
- In vivo xenograft tumor volumes (KM12 ± dox-shWRN1/C911). *n=5/5/4/4.*
- FM-HCR MMR-activity reporter (% reporter expression). *3×.*

> **Replication metadata is first-class Observed data.** For every assay node, record the (biological replicate, technical replicate, n) structure and the statistical test used. This is exactly what lets the encoding check that each claim's *scope of inference* (population-level vs measurement-event-level) is licensed by its replication structure — the pseudoreplication discipline. It is also where the discipline could *flag* an over-reach if one existed.

### 2.2 Derived — analyses

Two tiers (full method→handling table in §5):
- **Institution-recomputable** (the Eigenius statistics institution can re-run from O-* data and the kernel can attest the result matches the paper): differential-dependency mean differences, Wilcoxon rank-sum / signed-rank, two-tailed *t*, two-way ANOVA, Spearman correlation, BH-FDR, the cross-data-source linear-regression normalization, the gene-loss-vs-MSI linear models, PPV/sensitivity at the −0.5 threshold, least-squares-means contrast tests.
- **Linked external execution** (we do *not* reimplement; we link the github code's run as a typed `bench:ToolArtifact` boundary — Observed-grade provenance, not recomputed): CERES, DEMETER2, Limma empirical-Bayes moderated *t* (could later be an institution), edgeR/TMM, voom, GSEA/fgsea, lme4 mixed model, MSI classification (MSMuTect/MSIClass family).

### 2.3 Declared — hypotheses, definitions, design, domain rules

- **Hypotheses (Declared):** H1 "MSI/dMMR may create vulnerabilities"; the mechanistic hypothesis "WRN loss with MSI ⇒ DNA damage"; the telomere hypothesis (tested and *rejected* — encode the rejection).
- **Operational definitions / thresholds (Declared methodological resources — the "warranted decisions"):**
  - *MSI status*: classify by #deletions + fraction-in-MS, averaged across sources after linear-regression normalization; MSI / MSS / indeterminate.
  - *MMR loss*: a gene is inactivated if mutated (deleterious) ∨ deleted (log2 CN < −1) ∨ low-expressed (log2 TPM < 1); MMR-loss if any of MSH2/MSH6/MLH1/PMS2 inactivated.
  - *"Dependent"*: mean(CRISPR, RNAi) dependency < −0.5 (scores normalized: 0 = neg-control median, −1 = pan-essential median).
  - *TP53 functional status*: nutlin-3 sensitivity + CTD2 + p53-target expression signature.
  - *POLE status*: damaging / hotspot-missense / other from `Variant_Annotation`.
  - *Box-plot conventions* (hinges = 25th/75th, whiskers = 1.5·IQR) — Declared presentation rule.
- **Experimental design decisions (Declared, each with a rationale — these are the discipline's sweet spot):**
  - Negative controls (intergenic sgCh2–2/4; shRFP), pan-essential controls (sgPOLR2D/MYC; shPSMD2/RPS6).
  - **C911 seed control** (shWRN1-C911: nt 9–11 complemented, seed preserved) to rule out shRNA seed off-target [16].
  - **sgWRN-EIJ** (exon–intron junction) silences endogenous but not exogenous WRN cDNA ⇒ the rescue logic that establishes on-target effect.
  - **Exonuclease-dead (E84A) / helicase-dead (K577M) / dual** WRN cDNA [14] ⇒ the structure-function dissection.
  - HCT116 Ch3+5 MMR-restoration model [22] and the MLH1-KO re-sensitization control.
- **Domain rules imported from literature (Declared, each citing a reference — §3):** the synthetic-lethality definition [1]; p53-S15 as an ATR/ATM DDR readout [17]; DSBs toxic independent of p53 [19]; WRN nucleolar↔nucleoplasm relocalization on damage [20]; Sgs1/dMMR homeologous-recombination dependency in yeast [26]; etc.

### 2.4 Verified — and the checkable tier

- No Lean-style proofs in the biology. Do **not** invent VerifiedResources for wet-lab claims.
- **The checkable tier is institution-recomputed Derived (§2.2 tier 1).** When the statistics institution recomputes (e.g.) the Spearman rho or a Wilcoxon P from O-* data and the kernel attests it matches the paper, that node carries an `IsDerivedAs` warrant grounded in a re-runnable computation — the audit chain closes to raw data. This is the manifesto demonstration; prioritize it for the headline statistics in Phase 1.
- **One genuine formal opportunity:** the *synthetic-lethality logical schema* — `∀ gene g, context m: SelectivelyEssential(g, m) ⇐ ...` — and the *biomarker implication* could be authored as kernel-checkable propositions (Declared axioms), letting the per-claim warrants compose through them. Worth a small EigenTT formalization; not required for Phase 1.

---

## 3. Literature references → the exact claim each supports

The user requirement: be explicit about *what* each reference warrants. Each becomes a `reflection:DeclaredResource` (a literature warrant) whose `canonical_proposition` is the specific imported claim, cited by the node that uses it.

| Ref | Citation | Exact claim imported into this paper |
|---|---|---|
| 1 | Chan & Giaccia 2011 | *Definition/strategy:* synthetic-lethal interactions can be exploited for cancer therapeutics. |
| 2 | Ivy et al. 2016 | DNA-repair processes are attractive synthetic-lethal targets (many cancers have impaired repair). |
| 3 | Brown et al. 2016 | PARP-1 inhibitors succeed in HR-deficient cancers (proof-of-concept for the approach). |
| 4 | Kim et al. 2013 | MSI = hypermutable indel-at-microsatellite + SNV state; ~15% of colon; arises via Lynch (MSH2/MSH6/PMS2/MLH1 germline) or somatic MLH1 promoter hypermethylation. |
| 5 | TCGA gastric 2014 | MSI in ~22% of gastric cancers. |
| 6 | Kunitomi 2017 | MSI in ~20–30% of endometrial cancers. |
| 7 | Pal et al. 2008 | MSI in ~12% of ovarian cancers. |
| 8 | Le et al. 2017 | dMMR predicts solid-tumor response to PD-1 blockade. |
| 9 | Overman et al. 2018 | ICB benefit/limits in dMMR/MSI-H mCRC (45–60% non-response; toxicity). |
| 10 | Meyers et al. 2017 | Project Achilles CRISPR screen; **CERES** copy-number-corrected essentiality; pan-essential-median = −1 normalization. |
| 11 | McDonald et al. 2017 | Project DRIVE RNAi screen; also the *RPL22L1 dependency via RPL22 inactivation in MSI* claim. |
| 12 | Barretina/CCLE 2012 | CCLE; NGS-based MSI quantification; MSI classifications from CCLE Phase II. |
| 13 | Iorio et al. 2016 | PCR-based MSI phenotyping (concordance check). |
| 14 | Swanson et al. 2004 | WRN has separable 3′→5′ exonuclease & helicase functions; E84A/K577M missense mutants lack the respective activity. |
| 15 | Rossi et al. 2010 | WRN roles in genome-integrity (repair/replication/telomere); Werner-syndrome phenotype. |
| 16 | Buehler et al. 2012 | **C911** seed-preserving control design for siRNA/shRNA off-target. |
| 17 | Loughery et al. 2014 | p53-Ser15 phosphorylation is an ATR/ATM DDR target marking p53 activation. |
| 18 | Shiloh & Ziv 2013 | ATM-mediated DSB response activates p53 / anti-proliferative signaling. |
| 19 | Nowsheen & Yang 2012 | DSBs are toxic independent of p53 status (explains p53-impaired-MSI sensitivity). |
| 20 | Bendtsen et al. 2014 | WRN relocalizes nucleolus→nucleoplasm in response to DNA damage. |
| 21 | Billingsley 2014 | POLE-mutation hypermutation context. |
| 22 | Haugen et al. 2008 | HCT116 Ch3+5 MMR-restoration model (MLH1/MSH3); MSH3-loss instability. |
| 23 | Sidorova 2008 | WRN in DNA replication; insertion-deletion loops as substrates. |
| 24 | Spies & Fishel 2015 | MMR during homologous/homeologous recombination. |
| 25 | Opresko et al. 2009 | WRN processes mobile D-loops (branch migration/degradation). |
| 26 | Myung et al. 2001 | *Yeast:* dMMR creates a dependency on Sgs1 (WRN/BLM homolog) to resolve homeologous D-loops — the key mechanistic analogy. |
| 27 | Aggarwal et al. 2013 | WRN helicase previously nominated as a druggable target. |
| 28 | Lebel & Monnat 2018 | Werner-syndrome manifestations require decades (risk/benefit argument). |
| 29 | Behan, Iorio, Picco et al., *Nature* **568**:511–516 (2019), doi:10.1038/s41586-019-1103-9 | Companion Sanger / Project-Score genome-scale CRISPR study (same *Nature* issue, 7753) independently supporting WRN dependency in MSI. |
| 30 | Tsherniak et al. 2017 | Cancer Dependency Map concept/utility. |
| 31 | DepMap Achilles 18Q4 figshare | The CRISPR data fileset (O-ACHILLES provenance). |
| 32 | McFarland et al. 2018 | **DEMETER2** RNAi-dependency model. |
| 33 | DEMETER2 figshare | The reprocessed DRIVE data (O-DRIVE provenance). |
| 34 | CCLE/GDSC 2015 | DepMap/CCLE omics provenance; cross-dataset agreement. |
| 35 | Giacomelli et al. 2018 | TP53-status calling (p53-target signature; landscape of TP53 mutations). |
| 36 | Ritchie et al. 2015 | **Limma** — differential analysis via empirical-Bayes moderated *t*. |
| 37 | Benjamini & Hochberg 1995 | **BH-FDR** Q-value method. |
| 38 | Robinson & Oshlack 2010 | **TMM** library-size normalization. |
| 39 | Robinson et al. 2010 | **edgeR** (calcNormFactors). |
| 40 | Law et al. 2014 | **voom** mean-variance modeling for RNA-seq. |
| 41 | Subramanian et al. 2005 | **GSEA** method. |
| 42 | Sergushichev 2016 | **fgsea** fast implementation. |
| 43 | Liberzon et al. 2015 | MSigDB **Hallmark** gene-set collection (O-HALLMARK). |
| 44 | Boj et al. 2015 | Organoid establishment protocol. |
| 45 | Shibue et al. 2012 | IF protocol. |
| 46 | Lenth 2016 | **lsmeans/contrast** least-squares-means tests. |
| 47 | Dejmek et al. 2009 | WRN/fibrillarin IF protocol. |
| 48 | Bates et al. 2015 | **lme4** linear mixed-effects models (xenograft growth). |
| 49 | Nagel et al. 2014 | **FM-HCR** multiplexed DNA-repair reporter assay. |

(Refs 1–3, 14–20, 23–28 are *domain-rule* warrants; 10–13, 21–22, 29–35 are *data/biology* warrants; 36–49 are *method* warrants.)

---

## 4. Datasets to fetch and from which repository

| Dataset | Repository | Accessor | Used for |
|---|---|---|---|
| Achilles 18Q4 (CERES) | figshare | `7270880` [31] / depmap.org | O-ACHILLES — CRISPR dependency |
| DRIVE (DEMETER2) | figshare | doi:10.6084/m9.figshare.6025238.v4 [33] | O-DRIVE — RNAi dependency |
| DepMap 18Q4 omics (expr/CN/mutation) | DepMap | depmap.org [12,34] | O-OMICS — MMR/POLE/TP53/biomarker calling |
| CCLE RPPA | DepMap/CCLE | depmap.org | O-RPPA — MSH2/MSH6 protein |
| CCLE Phase-II MSI features | CCLE [12] | (via DepMap) | O-MSI — MSI classification inputs |
| GDSC nutlin-3 / CTD2 | cancerrxgene.org / ocg.cancer.gov/programs/ctd2 | [35] | O-TP53ANN — TP53-status inputs |
| mRNA-seq (WRN-KO) | GEO | **GSE126464** | O-MSEQ — GSEA |
| MSigDB Hallmark | MSigDB | [43] | O-HALLMARK — GSEA gene sets |
| WRN-manuscript curated inputs | figshare | doi:10.6084/m9.figshare.7712756.v1 | O-WRNFIG — exact analysis inputs |
| Analysis code | GitHub | cancerdatasci/WRN_manuscript | O-CODE — defines Derived pipelines |
| All materials portal | DepMap | depmap.org/WRN | cross-reference |

**Fetch priority for Phase 1:** O-ACHILLES, O-DRIVE, O-OMICS, O-MSI, O-WRNFIG, O-CODE — these alone support the entire computational-discovery spine and its recomputation.

External database cross-links to encode on entities (not fetched, linked): HGNC/Entrez gene IDs (WRN, MLH1, MSH2/3/6, PMS2, TP53, POLE, KRAS, BRAF, RPL22/RPL22L1), UniProt (WRN Q14191), Cellosaurus IDs for cell lines, ATCC/RIKEN/HSRRB sources, Addgene plasmid IDs (#78166, 46035–46038, 8453), GEO/figshare DOIs.

---

## 5. Computational institutions vs linked external executions

| Method (ref) | Eigenius handling | Rationale |
|---|---|---|
| Wilcoxon rank-sum / signed-rank | **Statistics institution** (recompute) | Standard nonparametric tests; recomputable from O-* → attestable warrant. |
| Two-tailed *t*-test, two-way ANOVA | **Statistics institution** | Already in scope; recompute the validation/mechanism P-values. |
| Spearman correlation | **Statistics institution** | WRN-dep ~ #MS-deletions; recompute rho/P. |
| Benjamini–Hochberg FDR [37] | **Statistics institution** (multiple-testing) | Q-values; recompute over the gene-level P set. |
| Linear regression normalization (MS-deletion cross-source) | **Statistics institution** | Recompute the scale/offset adjustment from O-MSI. |
| Gene-loss-vs-MSI linear models (Ext 9a) | **Statistics institution** | Recompute coefficients/P for the "no single gene explains WRN" claim. |
| PPV / sensitivity at −0.5 threshold | **Statistics institution** (or thin derived metric) | Recompute the biomarker predictivity table. |
| Limma empirical-Bayes moderated *t* [36] | **Linked external** now; *candidate institution later* | Moderated-*t* is a well-defined estimator; reimplementing is a real but bounded effort — flag as a future institution. For now link the R execution. |
| CERES [10] | **Linked external execution** | Upstream essentiality model; treat O-ACHILLES as Observed; link the producing pipeline as a `bench:ToolArtifact` boundary. |
| DEMETER2 [32] | **Linked external execution** | Same, for O-DRIVE. |
| GSEA / fgsea [41,42] | **Linked external execution** | Permutation enrichment; link the run, record NES/P as Derived-by-linked-tool. |
| edgeR/TMM [38,39], voom [40] | **Linked external execution** | RNA-seq normalization pipeline. |
| lme4 mixed model + LRT [48] | **Linked external** now; *candidate institution later* | Xenograft growth-rate interaction; reimplement later if mixed-models become an institution. |
| MSI classification (CCLE Phase II; MSMuTect/MSIClass) | **Linked external execution** | Upstream classifier; O-MSI features are Observed, the class call is linked-Derived. |
| lsmeans/contrast [46] | **Statistics institution** (extends current contrast support) | The IF MSI-vs-MSS contrast tests; aligns with existing `stats` contrast machinery. |

**Pattern:** anything the statistics institution can re-run from fetched data → recompute (attestable). Anything that is an upstream/bespoke pipeline → link the external execution as a typed `bench:ToolArtifact` (Observed-grade provenance, the D50 §9 boundary). The split is also a prioritized roadmap for *which institutions to build next* (Limma moderated-*t*, mixed models, GSEA) if the platform wants more of this paper recomputable.

### 5.1 Recompute fidelity policy (tolerance) — *resolved 2026-06-12*

A recomputed warrant compares an Eigenius-computed value against the paper's reported value. "Matches" is judged **per quantity class**, and the **binding** check is always preservation of the *qualitative scientific claim* (sign, ranking, threshold-crossing); numeric agreement is a secondary, class-dependent check. Everything is computed against a **pinned snapshot** — data version + library/algorithm versions + RNG seeds, all recorded as Observed provenance — so "within tolerance" is itself reproducible.

| Class | Quantities | Acceptance criterion |
|---|---|---|
| **A** — deterministic on pinned snapshot | rank-sum / signed-rank statistics, counts, BH ordering, threshold-based PPV / sensitivity | Test statistic **exact**; qualitative call exact; P/Q within numerical-method epsilon. |
| **B** — continuous effect sizes on pinned snapshot | Spearman/Pearson rho, mean dependency differences, log-fold-changes, contrast estimates | **Relative tolerance ≤ 2%** *or* within the reported significant figures (whichever is looser); **sign and threshold relation exact**. |
| **C** — significance values (P/Q), esp. extreme/bounded | Q = 4.8e-24 / 1.5e-45, P < 2.2e-16, etc. | Compared on **log10 scale**: agreement within **~1 order of magnitude** AND identical qualitative verdict (crosses α/FDR threshold; preserves ranking — e.g. WRN stays the top hit). For "P < X" floor reports: **one-sided** check (recompute ≤ X). Point-matching tiny P-values is explicitly *not* required. |
| **D** — statistics on upstream-linked outputs | anything computed on CERES/DEMETER2 scores or MSI class-calls | Class-B/C tolerances **widened** to absorb upstream version drift (record the pinned upstream version); the qualitative conclusion is binding. |
| **E** — permutation / stochastic | GSEA NES / P | NES within **≤ 5%** relative at a fixed seed + permutation budget; enrichment direction + significance verdict exact; P within Monte-Carlo resolution. |

**A discrepancy is a result, not a failure.** A recompute that violates its class tolerance *or* flips the qualitative claim does **not** silently pass — the chain records the divergence (a flagged / `Verdict::Fails` node carrying claim, paper-value, recomputed-value, class, verdict). That is precisely the discipline working: surfacing a gap between the published number and what the data re-yields. (Open knob for review: the Class-B 2% and Class-E 5% bands are first proposals; tighten/loosen per the snapshot's observed reproducibility.)

---

## 6. Domain ontology to author

A new benchmark/publication module — provisionally `onco` (cancer-dependency genomics) — on `bench-core`, declaring the *nouns* the propositions talk about. Thin, like `mol`:

- **Entities:** `onco:CellLine` (Observed; Cellosaurus/lineage/source), `onco:Gene` (HGNC/Entrez/UniProt), `onco:Lineage`, `onco:Perturbation` (sgRNA/shRNA, with sequence + vector + on/off-target metadata), `onco:Construct` (WRN cDNA variants: WT/E84A/K577M/dual).
- **Measurements (on `bench:Measurement`):** `onco:DependencyScore` (assay ∈ {CRISPR-CERES, RNAi-DEMETER2, aggregate}; value; normalization frame), `onco:ViabilityScore`, `onco:FociCount`, `onco:IFIntensity`, `onco:TumorVolume`, `onco:Expression`, `onco:CopyNumber`, `onco:ProteinAbundance`.
- **Status predicates (Declared classifications — Props over cell-line IRIs):** `onco:MSI(c)` / `onco:MSS(c)` / `onco:MSIindeterminate(c)`; `onco:MMRloss(c)`; `onco:TP53intact(c)` / `onco:TP53impaired(c)`; `onco:POLEdamaging(c)`.
- **Biology predicates (the reasoning targets):** `onco:SelectivelyEssential(gene, context)`, `onco:SyntheticLethal(gene, context)`, `onco:Biomarker(feature, dependency)`, `onco:RequiresActivity(gene, activity)` (helicase vs exonuclease), `onco:InducesDSB(perturbation, context)`, `onco:ActivatesP53(perturbation, context)`.
- **Reuse:** `bench:ToolArtifact` for every linked external execution; `bench:Dataset` for the fetched datasets; the statistics institution's `SampleSetResource` / `StatisticalAnalysisPlan` for recomputed tests (carrying the replication structure → scope-of-inference check).

Per-paper specifics (the WRN argument's concrete predicates and rules) are authored as the paper's *chain content*, not the base `onco` module — mirroring the per-task vocabulary discipline.

---

## 7. The logical dependency graph (edge list)

Nodes from §1; each edge `A ⇐ B` reads "A is justified by B". This is the `JustificationTerm` skeleton.

- `C-MAIN ⇐ {C-VAL, C-VIVO, C-HELICASE, C-MECH, C-MMR}` — the thesis composes validation, in-vivo, structure-function, mechanism, and causal dissection.
- `C-WRN ⇐ {D-DIFF(Achilles), D-DIFF(DRIVE)}` — independent CRISPR+RNAi replication (an Artemov `Sum`-like "either screen warrants it", but here joint = stronger).
- `D-DIFF ⇐ {O-ACHILLES|O-DRIVE, O-MSI, DEF-MSI, M-LIMMA, M-BH}` — differential dependency rests on the data, the MSI labels, the MSI definition, and the Limma+BH methods.
- `D-RECQ ⇐ {O-*, DEF-MSI, M-WILCOXON}`; `D-BIOM ⇐ {O-*, DEF-DEPENDENT, biomarker-method}`; `D-REFINE ⇐ {O-MSI, O-*, M-SPEARMAN}`.
- `C-VAL ⇐ {viability Observed nodes, M-ANOVA, ctrl-design Declared}`; `C-VAL ⇐ D-ONTARGET ⇐ {rescue Observed, sgWRN-EIJ Declared, C911 Declared[16]}`.
- `D-HELICASE ⇐ {rescue-by-variant Observed, construct-design Declared[14]}`.
- `C-VIVO ⇐ {xenograft Observed, M-LME4-LRT, C911 Declared[16], organoid Observed}`.
- `C-MECH ⇐ {GSEA(O-MSEQ,O-HALLMARK), IF Observed, M-CONTRAST, foci Observed, FISH Observed, rule[17],rule[18],rule[19],rule[20]}`; includes the **rejected** telomere sub-hypothesis (FISH showed diffuse, not telomeric, DSBs).
- `C-MMR ⇐ {HCT116-Ch3+5 Observed[22], MLH1-KO Observed, FM-HCR Observed[49], M-ANOVA}` and `⇐` the Declared interpretation that dMMR contributes-but-not-fully (citing analogy [26]).
- Cross-cutting Declared axioms: `AX-SL` (synthetic-lethality schema [1]), `AX-DSB-TOX` (DSBs toxic independent of p53 [19]) — used to license `C-MMR`'s "p53-impaired MSI still sensitive" sub-claim.

Encoding each edge as a kernel-checked `JustifiedBy` certificate is the Phase-by-phase work; the headline is that `C-MAIN`'s certificate composes the entire graph, and the data-backed leaves (`D-DIFF`, `D-REFINE`, …) carry *recomputed* warrants.

---

## 8. Encoding plan — layers, files, phasing

**Layering (on the existing bootstrap chain):**
`core → reflection → … → reasoning → bench-core → harness → onco → wrn-paper-vocab → wrn-paper-chain`

- `experiments/publications/wrn-helicase/chain/01-onco.esl` — the `onco` domain module (§6).
- `experiments/publications/wrn-helicase/wrn-vocab.esl` — paper-specific predicates + Declared definitions/thresholds + literature-rule DeclaredResources (§2.3, §3).
- `experiments/publications/wrn-helicase/datasets.esl` — the O-* `bench:Dataset` / Observed resources with provenance + external-DB links (§4).
- `experiments/publications/wrn-helicase/chain-phase1.esl … chain-phase5.esl` — the reasoning chain per phase (§1 spine).
- `experiments/publications/wrn-helicase/fetch/` — scripts + manifests to pull O-* from the repositories (§4), with checksums (content-addressed provenance).
- A validation test (parallel to `sab*_tracer.rs`) that builds the chain and (Phase 1) drives the statistics institution to recompute the headline numbers and assert agreement.

**Phases (each independently committable + checkable):**

- **Phase 0 — fetch + onco module.** Pull Phase-1 datasets (§4 priority), author `chain/01-onco.esl` + `wrn-vocab.esl` + `datasets.esl`; validate they round-trip (smoke test, like the bench-ontology test). *Decision needed: how/whether to vendor the data vs. link by accession (§9).*
- **Phase 1 — computational discovery spine (the headline).** *First increment landed 2026-06-12:* `chain/01-onco.esl` (module) + `chain/05-phase1-discovery.esl` encode `H1`, the two `D-DIFF` recompute artifacts, the Declared discovery rule, `C-WRN` (`SelectivelyEssential(WRN, MSI)`), and `D-REFINE` (carrying the corrected `n=51` + finding F1) — both conclusions validate to `Holds` (`crates/eigenius-reasoning/tests/wrn_phase1.rs`). Statistical claims carried as recompute Derived artifacts (IsDerivedAs via ProgramTrace) against the pinned snapshot. *Second increment landed 2026-06-12:* `D-RECQ` (WRN uniquely MSI-selective among RecQ helicases — recomputed WRN P=1.1e-8, others n.s.; RECQL4 absent from the 18Q4 CERES snapshot) and `D-BIOM` (MSI a strong biomarker — recomputed common-lineage PPV=0.73, sensitivity=1.00) both validate to `Holds`; and a Phase-1 `bench:TaskOutput` deliverable (`wrn:discovery_finding`, `deliverable_kind="prose"`) cites the four conclusions via `reasoning_chain`. The full computational-discovery spine is now chain-resident.

*Recompute-upgrade increment 1 landed 2026-06-12:* `wilcoxon_rank_sum` added to the statistics institution's numerics (`crates/eigenius-statistics/src/numerics.rs`, tie-corrected normal approximation with continuity correction), validated to reproduce the **real** WRN MSI-vs-MSS dependency comparison (37 vs 91 common-lineage values from the pinned snapshot) at P~4e-13 — matching the paper's 4.2e-13 (§5.1 Class-C). 67 statistics tests pass. *Recompute-upgrade increment 2 landed 2026-06-12:* `validate.rs` IID dispatch now branches on `variance_assumption = RankBased`/`NonParametric` → `wilcoxon_rank_sum`, and the two-sample canonical proposition (`¬(mean_diff_of(s)=0)` / `lt(mean_diff_of(s),0)`) is derived in Step 6.5 for the IID dispatch (previously unwired). End-to-end test `crates/eigenius-statistics/tests/wilcoxon_wrn.rs`: a committed `StatisticalAnalysisPlan(IID, RankBased)` on the real 37-vs-91 WRN values recomputes Wilcoxon to **Holds**, reproduces P~4e-13, and emits a `StatisticalAnalysisResult` carrying the derived proposition (so D49 admits `IsDerivedAs`). 13 statistics suites pass, clippy clean.

*Recompute-upgrade increment 3 landed 2026-06-12 — the two-institution composition.* `experiments/publications/wrn-helicase/wrn-phase1-recompute-{plans,conclusions}.esl` authors the WRN MSI/MSS `SampleSet` + `StatisticalAnalysisPlan(RankBased, OneSidedWitnessed)` + a directionality `ImpossibilityWitness` + a declared statistical→domain bridge + `concl_wrn_selective_recomputed`. Test `crates/eigenius-statistics/tests/wrn_phase1_recompute.rs`: the **statistics** institution recomputes the Wilcoxon (real data, Holds, P~4e-13) and emits an `IsDerivedAs`-bearing `StatisticalAnalysisResult`; the **reasoning** institution then type-checks `C-WRN` (`SelectivelyEssential(WRN, MSI)`) against it via the bridge → **Holds**. The two never call each other — they compose through the shared chain witness index. **C-WRN's warrant is now kernel-recomputed, not agent-attested.** 14 statistics + 9 reasoning suites pass.

Integration note for anyone dispatching institutions directly: the raw `institution.query()` returns the derivation *before* the kernel's AutoOnLoad commit path runs `finalize_emitted_derivation` (which stamps `reflection:InstitutionEmittedDerivation` + `DerivedResource` — the markers the D49 witness emitter walks). A direct-dispatch test must replicate that finalization before committing the result.

*Recompute-upgrade increment 4 landed 2026-06-12 — Spearman / D-REFINE.* `spearman_correlation` added to numerics (rank-correlation + t-approximation; unit test reproduces the real rho = -0.7412 on the 51 MSI pairs — the corrected n). `validate.rs` Paired dispatch branches on `RankBased`/`NonParametric` → Spearman, with a new correlation canonical-proposition (`¬(spearman_rho(s)=0)` / `lt(spearman_rho(s),0)`) wired in Step 6.5; new axiom `stats:spearman_rho`. Institution test `crates/eigenius-statistics/tests/spearman_wrn.rs`: a committed `StatisticalAnalysisPlan(Paired, RankBased)` on the real (#MS-deletions, WRN-dep) pairs recomputes the correlation to **Holds**, reproduces rho = -0.74, and emits an `IsDerivedAs`-bearing result. 15 statistics + 9 reasoning suites pass, clippy clean.

*Recompute-upgrade increment 5 landed 2026-06-12 — the D-REFINE two-institution composition.* `wrn-phase1-recompute-{plans,conclusions}.esl` gains the D-REFINE half: a `mutator_load_directionality_witness` (`ImpossibilityWitness`), `wrn_corr_sampleset` carrying the real 51 (#MS-deletions, WRN-dep) pairs as a `stats:Paired` sample set, `wrn_corr_plan` (`Paired`, `RankBased`, `OneSidedWitnessed`), a declared statistical→domain bridge `bridge_mutator_load` (`stats:lt(stats:spearman_rho("…wrn_corr_sampleset"),0.0) -> onco:DependencyCorrelatesWithMutatorLoad("WRN","MSI")`), and `concl_refine_recomputed`. The composition test `crates/eigenius-statistics/tests/wrn_phase1_recompute.rs` (now `wrn_warrants_kernel_recomputed`) recomputes **both** plans through the statistics institution, commits both `IsDerivedAs`-bearing results, and the reasoning institution type-checks **both** `concl_wrn_selective_recomputed` (C-WRN) and `concl_refine_recomputed` (D-REFINE) to **Holds** — the two institutions still never call each other. **Both WRN headline warrants are now kernel-recomputed and chain-composed.** 16 statistics + 9 reasoning suites pass.

*Recompute-upgrade increment 6 landed 2026-06-12 — D-RECQ + D-BIOM recomputed; recorded `dd_*` retired; chain consolidated.* The institution-recomputable tier (D50 §2.2) is now fully kernel-recomputed:
- **D-BIOM** — a new `stats:ClassificationAnalysisPlan` capability in the statistics institution (D52 §2.2): a standalone plan + `stats:classification_threshold`/`min_ppv`/`min_sensitivity` properties + `stats:ppv`/`stats:sensitivity` axioms + `numerics::classification_metrics` (threshold-classifier confusion counts). Class-based early dispatch (mirroring `MethodComparisonAnalysisPlan`) routes to `recompute_classification_quality_claim`, which emits two `StatisticalAnalysisResult`s (`:result:ppv`, `:result:sensitivity`) carrying `stats:ge(stats:ppv(s),0.7)` / `stats:ge(stats:sensitivity(s),0.9)`. Over the **same** `wrn_dep_sampleset` (37 MSI = test-positive, 91 MSS) at threshold −0.5 it recomputes PPV = 27/37 = 0.73 and sensitivity = 27/27 = 1.00 (matching the retired artifact exactly; the recorded "37 MSI" cohort confirmed against the slices). A declared bridge composes both into `onco:StrongBiomarker(MSI, WRN_dependency)`.
- **D-RECQ** — `wrn_recq_sampleset` (the real Achilles CRISPR-CERES WRN gene-effect, **32 MSI / 413 MSS** all-lineage values, extracted + checksummed from the pinned slices; reproduces WRN Wilcoxon P = 1.1e-8) + `wrn_recq_plan` → kernel-recomputed WRN selectivity. The **uniqueness** ("only WRN among RecQ helicases") is a *Declared* rule, not a derived warrant: a null is not derivable, so the non-significance of BLM (P=0.65), RECQL (0.58), RECQL5 (0.11) — all kernel-computed against the same snapshot, RECQL4 absent — is recorded as the rule's rationale and the explicit scientific judgment that adequate-n non-significance ⟹ no MSI-selective dependency. `concl_recq_recomputed` composes the WRN derived result with that rule.
- **Retired**: the recorded `refine_spearman`, `dd_recq`, `dd_biomarker` ToolArtifacts and their recorded conclusions (`concl_refine`/`concl_recq`/`concl_biomarker`).
- **Chain consolidated**: `chain/05-phase1-discovery.esl` is now the *narrative/deliverable* layer (datasets, H1, the linked-external two-screen D-DIFF + discovery rule + `concl_wrn_selective`, and the `bench:TaskOutput` citing the four recomputed conclusions), stacked on top of `wrn-phase1-recompute-{plans,conclusions}.esl` (all kernel-recomputed warrants). One comprehensive test (`wrn_phase1_recompute.rs::wrn_warrants_kernel_recomputed`) builds the full stack, recomputes all four plans (5 results incl. the classification plan's two), commits them, and type-checks all four recomputed conclusions + the linked-external two-screen `concl_wrn_selective` + the deliverable. The reasoning-crate `wrn_phase1.rs` is deleted (subsumed). 70 statistics lib + all statistics integration + 9 reasoning suites pass, clippy clean.

*Provenance increment 7 landed 2026-06-12 — Tier-1 extraction pin.* Closed the one previously-uncommitted link in the audit chain: the projection from each pinned slice to the `stats:sample_set_value` arrays inlined in `wrn-phase1-recompute-{plans,conclusions}.esl`. (1) Committed the canonical extractor `extract/extract_samplesets.py` (single source of truth: per-SampleSet source slice + enforced sha256 + column + filter + sort + grouping; `--check` re-derives and diffs vs the ESL, `--emit` regenerates). (2) Recorded the recipe **in-chain** via new `bench:extracted_from_slice`/`extracted_from_sha256`/`extraction_columns`/`extraction_filter`/`extraction_recipe` properties (declared in `bench-core.esl`, domain `reflection:ObservedResource`) on all three SampleSets. (3) Wired it into the suite as an `#[ignore]`d test (`crates/eigenius-statistics/tests/wrn_sampleset_pin.rs`) that shells to `--check` — skips gracefully when the gitignored slices / `python3` are absent, fails only on drift. All three arrays re-derive byte-for-byte (128 / 102 / 445 values). This guards against silent corruption of the inlined evidence and makes the slice→SampleSet step reviewable. **Tier-2 follow-up (designed, not built — [docs/design/d53-large-data-tracking.md](../../../../docs/design/d53-large-data-tracking.md)):** lift the recipes onto the runtime substrate (D26) — a content-hash-pinned external file + an on-chain `RuntimeScript` + a `DataExtractionPlan` AutoOnLoad gate that runs it and *emits* the SampleSet as a `DerivedResource` witnessed by a `RuntimeInvocation` (input hash + script hash + image digest + output hash), reclassifying it from Observed-with-recipe-sidecar to Derived-from-raw-Observed. Reuses the same machinery as Phase 2.5's limma; the one new primitive is the content-hash-pinned external file input.

The statistics institution now kernel-recomputes the entire institution-recomputable WRN tier (`C-WRN` Wilcoxon, `D-REFINE` Spearman, `D-RECQ` family Wilcoxon, `D-BIOM` PPV/sensitivity), all composing end-to-end into reasoning conclusions. The **only** remaining recorded boundary is `dd_achilles`/`dd_drive` (`TopDifferentialDependency` — the genome-wide limma moderated-*t* ranking, which a single-gene test cannot establish), correctly linked-external until Phase 2.5. Remaining: Phase 2.5 (limma moderated-*t*) for the exact D-DIFF Q-values. Encode `H1 → D-DIFF → C-WRN → D-RECQ/D-BIOM/D-REFINE`. **Recompute** via the existing statistics institution: the differential-dependency mean-differences + BH ordering + the *WRN-is-top-hit* ranking, the RecQ Wilcoxon Q-values, the Spearman correlations (rho=−0.74/−0.57), and the biomarker PPV/sensitivity. The **exact limma moderated-*t* Q-values** (4.8e-24 / 1.5e-45) are *partially* recomputed here — as the mean-difference/ranking/BH claim — with the moderated-*t* P linked-external until **Phase 2.5** makes it fully recomputed. Even partial, this phase *is* the manifesto demonstration: a published computational claim re-derived from public data, with the audit chain closed to raw screen scores. Linked-external for CERES/DEMETER2 upstream.
*Recompute-upgrade increment 8 landed 2026-06-13 — C-VAL wet-lab two-way ANOVA, kernel-recomputed (new NestedAnovaAnalysisPlan dispatch).* After securing the Nature per-figure Source Data (all 10 XLSX vendored + checksummed; see `data/MANIFEST.md`), the ED Fig 3b competition-assay two-way ANOVA was lifted from linked-external to kernel-recomputed. **New institution capability:** `stats:NestedAnovaAnalysisPlan` — the authors' `lm(value ~ is_WRN + guide)` nested fixed-effects model (group effect tested against the within-subgroup residual; our Factorial dispatch is *crossed*, so it couldn't express it). `numerics::nested_group_anova` (+ unit test), a standalone plan class + `subgroup_sizes_a/b` properties + QueryClass, and a class-based early dispatch emitting `stats:lt(stats:mean_diff_of(s), 0)` (group A below group B) — the same proposition shape as the two-sample dispatch, so existing bridges consume it. Reuses the `stats:IID` two-group SampleSet (group A = sgWRN arm, B = control arm) + the plan's subgroup partition. **Validated against the paper exactly** (`crates/eigenius-statistics/tests/nested_anova_wrn.rs`): KM12 (MSI) Holds reproducing P=2.7e-19, ES2 (MSS) Fails reproducing 0.37. **C-VAL recompute:** `wrn-phase1-recompute-{plans,conclusions}.esl` gains `viability_directionality_witness`, KM12 + OVK18 SampleSets (the real day-10 relative ratios, Tier-1-pinned from `wrn_sourcedata_EDFig3_MOESM6.xlsx`) + NestedAnovaAnalysisPlans, a 2-arg bridge composing both MSI lines into `concl_val_recomputed` (`SelectiveViabilityDependence(WRN, MSI)`; the MSS-spared side is the declared judgment — a null is not derivable). The linked-external `va_competition`/`concl_val` pair is **retired**; `C-MAIN` now cites `concl_val_recomputed`. Composition test recomputes 8 results (incl. the 2 nested ANOVAs) and validates `concl_val_recomputed`; all suites green (statistics 18, reasoning 11). The `NestedAnovaAnalysisPlan` dispatch is reusable for ED Fig 4 (cell-cycle/apoptosis) and ED Fig 10 (MMR-restoration), which share the `value ~ is_WRN + guide` design.

*Recompute-upgrade increment 9 landed 2026-06-13 — C-MECH DDR endpoints (cell-cycle arrest + apoptosis), kernel-recomputed.* The two downstream DSB-driven-DDR consequences were lifted from the linked-external mechanism narrative to kernel-recomputed, reusing increment 8's `NestedAnovaAnalysisPlan` dispatch (no new capability needed). From the pinned `wrn_sourcedata_EDFig4_MOESM7.xlsx` (Tier-1 checksummed): **ED Fig 4b** (cell-cycle distribution, %S-phase) and **ED Fig 4c** (Annexin-V total apoptosis), each recomputed for **all three MSI lines** present (KM12, SW48, OVK18 — SW48 is a new MSI line not previously on chain). `wrn-phase1-recompute-{plans,conclusions}.esl` gains a `cellcycle_directionality_witness` + 3 `cc_*` SampleSets/plans + `bridge_cellcycle` → `concl_cellcycle_recomputed` (`CausesCellCycleArrest(WRN, MSI)`), and an `apoptosis_directionality_witness` + 3 `apop_*` SampleSets/plans + `bridge_apoptosis` → `concl_apoptosis_recomputed` (`CausesApoptosis(WRN, MSI)`). **Directionality flip handled cleanly:** S-phase *falls* on arrest (group A = sgWRN below control), apoptosis *rises* (the SampleSet places control first as group A so mean_a < mean_b again) — both reduce to the same `stats:lt(mean_diff_of(s), 0)` proposition the existing bridges consume. **Reproduced the paper's two-way ANOVA exactly:** cell-cycle KM12 P=6.1e-7 / SW48 3.5e-4 / OVK18 2.6e-6; apoptosis KM12 3.4e-3 / SW48 3.6e-4 / OVK18 3.6e-5; the three MSS lines (SW620/SW837/ES2) are n.s. in both panels (recomputed to Fail — the declared 'spared' judgment, a null is not derivable), cited in the bridge rationales. **C-MECH strengthened structurally:** `mech_rule` went from `CausesDSBs → DSBDrivenLethality` (1 antecedent) to `CausesDSBs → CausesCellCycleArrest → CausesApoptosis → DSBDrivenLethality` (3 antecedents); `concl_mech` now discharges the DSB leg with the linked-external γH2AX ToolArtifact and the two DDR legs by **D54 lemma citation** of the recomputed conclusions — so 2 of the 3 mechanism legs are now kernel-recomputed (the DSB-marker IF remains the external-tool frontier). New `onco` predicates `CausesCellCycleArrest`, `CausesApoptosis`. Composition test recomputes **14 results** (incl. the 6 new nested ANOVAs) and validates both new conclusions; `concl_mech` (phase3) and `concl_main` (phase5) still `Holds`. Extractor `--check` re-derives all 12 SampleSets byte-for-byte. All suites green (statistics 18 integration + 71 lib, reasoning 11), clippy clean. **ED Fig 4d** (shRNA apoptosis, SW837+KM12 × 2 days) was *not* encoded — ED 4c already establishes MSI-selective apoptosis with three CRISPR-based MSI lines; 4d is shRNA corroboration in a subset, redundant for the warrant.

*Design finding — ED Fig 10c (MMR-restoration) reproduces EXACTLY from public data (recompute-findings F3); needs a crossed two-way ANOVA dispatch.* Per the authors' vendored analysis code (`data/WRN_manuscript/src/WRN_stats_calcs.Rmd:228-323`), the C-MMR viability contrasts are each a **crossed additive two-way ANOVA** `lm(value ~ CL + guide)` over a *pair* of conditions, testing the `CL` (MMR-context) main effect controlling for `guide` — the **same formula family** as C-VAL's `value ~ is_WRN + guide`, *not* the pooled interaction-contrast model first assumed. The only structural difference from increment 8's nested dispatch: here `guide` is **crossed** (shared shRNAs across both `CL` levels), so the residual pools CL×guide interaction (`df = N − #CL − #guide + 1`) rather than the nested `N − n_subgroups`. **The data is public and reproduces the paper exactly:** source = `wrn_sourcedata_EDFig10_MOESM12.xlsx` sheet **"ED Fig 10c"** (relative viability, **n = 6**), four HCT116 derivative blocks (Ch2 ∗ / Ch3+5+sgCh2-2 † / Ch3+5+sgMLH1-1 ‡ / Ch3+5+sgMLH1-2 §); using the **normalized** values, filtering to the **shWRN guides** (shWRN1+shWRN2, the bars ∗†‡§ mark), `lm(value ~ CL + guide)` testing CL reproduces **5.74e-20 / 3.26e-12 / 1.56e-16** vs the paper's **5.7e-20 / 3.3e-12 / 1.6e-16**. (An earlier pass mis-called this "blocked": it used the wrong sheet — "ED Fig 10f", n = 3, a duplicate-fill secondary panel — and tested all guides, diluting the shWRN rescue to ≈7e-5. The non-public `reformattedforstats.xlsx` is just a relabeled concatenation of these same published numbers.) **Landed in increment 10** (below).

*Recompute-upgrade increment 10 landed 2026-06-13 — C-MMR MMR-restoration, kernel-recomputed (new CrossedAnovaAnalysisPlan dispatch).* The last wet-lab warrant with reproducible public per-replicate data was lifted from linked-external. **New institution capability:** `stats:CrossedAnovaAnalysisPlan` — the authors' `lm(value ~ CL + guide)` with `guide` **crossed** (shared across both groups), the C-MMR analogue of increment 8's *nested* dispatch. `numerics::crossed_two_way_anova` (+ unit test reproducing F(1,21)=1187.5 / P=5.74e-20) tests the 2-level group main effect against the additive-model residual (`df = N − #group − #block + 1`, pooling the group×block interaction — distinct from nested's `N − n_subgroups`); a standalone plan class (reusing `subgroup_sizes_a/b`, here paired by index across groups) + class-based early dispatch + QueryClass gate, emitting the same `stats:lt(stats:mean_diff_of(s), 0)` proposition so existing bridges consume it. **Latent fix:** both the crossed and nested emitters now use the FisherSnedecor survival function `sf` instead of `1 − cdf`, which underflowed to 0 for extreme F (C-VAL's KM12 2.7e-19 / ED10c's 5.7e-20) — the verdict was always correct but the reported p was 0; now the true tiny p is reported. **C-MMR recompute:** `wrn-phase1-recompute-{plans,conclusions}.esl` gains `mmr_directionality_witness`, three Tier-1-pinned SampleSets from ED Fig 10c relative-viability (n=6, normalized, shWRN1+shWRN2): `mmr_rescue` (Ch2 vs Ch3+5+sgCh2-2, P=5.7e-20), `mmr_resens1`/`mmr_resens2` (the two MLH1-KO re-sensitization controls, P=3.3e-12 / 1.6e-16) + CrossedAnovaAnalysisPlans, a 3-antecedent `bridge_mmr_restoration` composing them into `concl_mmr_restoration_recomputed` (`RestorationPartiallyRescues(dMMR, WRN)`; "partially" = the declared modest-rescue qualifier). The linked-external `mmr_restoration` ToolArtifact is **retired**; `concl_mmr` (phase 5) now discharges its antecedent by **D54 lemma citation** of the recomputed conclusion. Composition test recomputes **17 results** (incl. the 3 crossed ANOVAs) and validates the new conclusion; `concl_mmr`/`concl_main` still `Holds`; extractor `--check` re-derives all 15 SampleSets byte-for-byte; all suites green (statistics 19 integration + 72 lib, reasoning 11), clippy clean. The full data-mapping detective work (display-vs-analysis spreadsheet, panel-letter drift, decoy sheet, symbol→condition decoding, raw-vs-normalized, the decisive shWRN-only subset) is recorded in recompute-findings F3.

- **Phase 2 — wet-lab validation.** *Landed 2026-06-13.* `C-VAL`, `D-ONTARGET`, `D-HELICASE` encoded in `chain/07-phase2-validation.esl` (stacks on the Phase-1 chain), validated by `crates/eigenius-reasoning/tests/wrn_phase2.rs` (all four conclusions `Holds`). **Key scoping finding:** Phase-2 statistics are the authors' *own* wet-lab assays (luciferase competitive-growth/viability two-way ANOVA — ED Fig 3b day-10 MSI 2.7e-19/1.2e-7 vs MSS 0.37/0.23 n.s., n=6; IF contrast-test-of-least-squares-means), whose raw data is in **no public slice we hold** (only Supp Table 1, the Phase-1 annotation, is vendored). So the wet-lab readouts are **linked-external** `bench:ToolArtifact`s citing the reported values (the same boundary `dd_achilles`/`dd_drive` occupy for limma) — *not* kernel-recomputed. The **encodable contribution** is the Declared experimental-design reasoning, now kernel-checkable: D-ONTARGET = the sgWRN-EIJ on-target rescue logic (`RescuesDepletion(WRN_cDNA_WT, sgWRN_EIJ) → OnTarget(WRN, MSI_viability)`, with sgWRN2-non-rescue as the specificity control in the rationale); D-HELICASE = the structure-function dissection (`FailsToRescue(WRN_cDNA_K577M, sgWRN_EIJ) → RequiresActivity(WRN, helicase)` and `RescuesDepletion(WRN_cDNA_E84A, …) → DispensableActivity(WRN, exonuclease)`); C-VAL = `SelectiveViabilityDependence(WRN, MSI)` from the multi-assay readout. New `onco` predicates: `RescuesDepletion`, `FailsToRescue`, `OnTarget`, `DispensableActivity`, `SelectiveViabilityDependence` (non-rescue modeled as a positive measured `FailsToRescue`, not a derived null). This phase exercises the Declared-rule + logical-composition side of the reasoning institution — the complement to Phase 1's statistics-recompute side. Optional later: fetch the competition-assay raw data (if a repository carries it) to upgrade the linked-external readouts to recomputed, and add `C-VIVO` (xenograft lme4 LRT + C911 seed-control logic) + the patient-derived-organoid arm.
- **Phase 2.5 — Limma moderated-*t* institution (high-leverage; upgrades Phase 1 + unlocks Phase 4).** Build the empirical-Bayes moderated-*t* as an Eigenius computation so `D-DIFF` and the Phase-4 RNA-seq differential expression become *fully* recomputed (exact moderated-*t* P/Q, not just mean-difference + BH). Two paths:
  - **(a) Native Julia institution** — reuses the existing D27 Julia runtime substrate (no new language runtime): empirical-Bayes variance shrinkage via `EBayes.jl`, design-matrix fitting via `GLM.jl`, base tests via `HypothesisTests.jl`. More Eigenius-native; carries a *fidelity-validation burden* (limma's `squeezeVar` / `fitFDist` prior-degrees-of-freedom estimation has a specific formulation the Julia stack must reproduce within the §5.1 Class-B band).
  - **(b) Wrapped R** — an R runtime substrate parallel to the Julia one; runs limma directly (faithful by construction, lowest fidelity risk) but adds a new language runtime.
  - **Recommended:** prototype **(a)**, using **(b)** as a *one-time calibration oracle* — validate the Julia moderated-*t* against R limma on the actual WRN differential-dependency data within Class-B tolerance before trusting it; fall back to (b) as the runtime only if (a) cannot reproduce limma there. One institution then serves both `D-DIFF` (Phase-1 upgrade) and Phase-4 RNA-seq differential expression.
- **Phase 3 + 4 — in vivo + mechanism.** *Landed 2026-06-13* (encoded together in `chain/08-phase3-invivo-mechanism.esl`, stacks on Phase 2; validated by `crates/eigenius-reasoning/tests/wrn_phase3.rs`, all five conclusions `Holds`). **C-VIVO**: `InVivoDependence(WRN, MSI)` (KM12 xenograft lme4-LRT + MSI patient-derived organoid, linked-external) and the **C911 seed-control logic** as a Declared rule — `SeedControlInert(WRN, xenograft_growth) → OnTarget(WRN, xenograft_growth)` (shWRN1-C911 preserves the off-target seed but is inert, so the effect is not seed-driven). **C-MECH**: `CausesDSBs(WRN, MSI)` (γH2AX/53BP1/pATM/Chk2 foci, linked-external) lifted by a Declared mechanism rule to `DSBDrivenLethality(WRN, MSI)` (rationale: GSEA G2/M+E2F down / apoptosis+p53 up, annexin V, p53-S15/p21 IF, DSBs toxic independent of p53); the **tested-and-rejected telomere hypothesis** as `NotViaTelomereDefect(WRN, MSI)` (telomere-FISH: diffuse chromosomal DSBs, no telomeric fusions/signal loss). **Recomputed sub-warrant:** the p53 dissection's "p53 contributes" half IS kernel-recomputed — `concl_p53_modulates` (`ModulatesDependence(TP53, WRN)`) from a new `p53_dep_sampleset` (23 p53-intact vs 13 p53-impaired MSI WRN-dep values, Wilcoxon, reproduces the paper's P=0.02 exactly) in the statistics layer, validated by the `wrn_phase1_recompute` composition test (now 6 results) and pinned by the Tier-1 extractor. New `onco` predicates: `InVivoDependence`, `CausesDSBs`, `DSBDrivenLethality`, `NotViaTelomereDefect`, `ModulatesDependence`, `SeedControlInert`. The rest of C-MECH (GSEA, IF contrasts, foci) is linked-external — the authors' own assays, raw data not vendored.
- **Phase 5 — causal dissection + thesis.** *Landed 2026-06-13* (`chain/09-phase5-synthesis.esl`, stacks on Phase 3; validated by `crates/eigenius-reasoning/tests/wrn_phase5.rs`). **C-MMR**: `ContributesToDependence(dMMR, WRN)` via a Declared causal-dissection rule (`RestorationPartiallyRescues(dMMR, WRN) → ContributesToDependence(dMMR, WRN)`) over the linked-external HCT116 Ch3+5 MMR-restoration + MLH1-KO re-sensitization readout — "partially" being the load-bearing qualifier (contributes but doesn't fully explain; cf. D-REFINE mutator load). New predicates `RestorationPartiallyRescues`, `ContributesToDependence`. **C-MAIN** (`SyntheticLethal(WRN, MSI)`): the capstone, reached by **modus ponens** over a Declared synthesis implication `SVD → IVD → RA → DL → CD → SyntheticLethal` applied to the five findings (C-VAL, C-VIVO, D-HELICASE, C-MECH, C-MMR). **Modeling note (corrected mid-encoding):** a proven phase conclusion is the *antecedent of this implication*, NOT an evidence atom — citing a `ReasoningSentence` via `DerivedEvidence` is the wrong abstraction (and the kernel refuses it: a sentence isn't an admitted `IsDerivedAs` witness). C-MAIN therefore discharges each antecedent by **inlining its leaf proof** (the same warrant the phase conclusion used; all admitted via traces, no institution needed). **Lemma citation landed 2026-06-13 ([docs/design/d54-reasoning-lemma-citation.md](../../../../docs/design/d54-reasoning-lemma-citation.md), implemented).** A proven `ReasoningSentence` is now citable as a *lemma*: `build_witness_index` admits it as a `Verified` witness keyed on its IRI (one kernel branch; soundness via the commit pipeline rejecting `Fails` sentences). **C-MAIN was converted from inlined leaf proofs to five lemma citations** (`verified(concl_val/concl_vivo/concl_helicase_required/concl_mech/concl_mmr, P)`) — still `Holds` (`wrn_phase5.rs`), the certificate collapsed, the modus-ponens spine unchanged. D54 also records the resolved design decisions (§4.1 Form A "citer restates P" + `default = proposition`; §4.2 witness category = **Verified**, with the `IsVerifiedAs → IsDerivedAs` coercion making it strictly dominant; §4.3 only `ReasoningSentence`s are lemmas — institution `Verdict`s carry no proposition so are never citable) and, on the consistency question, how it is logic-dependent (justification logic's per-category factivity, paraconsistent coexistence of conflicting justifications, and the realization-theorem decidability boundary that makes `qc_consistency_check` honestly `Undecidable` on the first-order fragment). Layered proof (lemmas → theorems) is now available platform-wide. The whole `H1 → … → C-MAIN` argument graph now type-checks end-to-end across Phases 1-5.

**Recommended first target: Phase 0 + Phase 1.** Self-contained, maximally recomputable, and it proves the thesis on its own.

### Wet-lab data availability (correction + tracked follow-up, 2026-06-13)

*Correction.* Phases 2-5 mark the wet-lab readouts **linked-external**, and the notes above say their raw data "is in no public slice we hold." That is true (we vendored only Supp Table 1) but understates availability. The paper's **Code/Data Availability** statement provides **source data with the paper** for nearly all of them — Figs. 2a–g, 3c, 4a,c,e,f and ED Figs. 3a,b,d, 4b–d, 5b,d,f, 6a,c,d,f,h, 7b,d,e, 8d, 10a–d,f — plus **GEO GSE126464** (mRNA-seq → GSEA, Fig 3a), the **Figshare DepMap bundle** (doi:10.6084/m9.figshare.7712756.v1), and **code at github.com/cancerdatasci/WRN_manuscript**. So "linked-external" here means **not-yet-fetched**, not unavailable; the source-data files are Nature/NIHMS downloads we simply haven't vendored.

*Tracked follow-up — upgrade the recomputable subset.* If the source data is vendored (checksummed via the Tier-1 pin / D53), the readouts split by what the **existing** statistics institution can recompute:
- **Recomputable now (Factorial/two-way-ANOVA dispatch):** the competitive-growth assay (`va_competition`, ED Fig 3b, n=6 biological replicates), cell-cycle %S-phase (ED 4b), apoptosis (ED 4c,d) — per-replicate values → kernel-recomputed two-way ANOVA, the same upgrade pattern as the p53 Wilcoxon. The rescue assays (Fig 2b,c) are competition readouts in the same family.
- **External-tool frontier (Phase 2.5 / D53, not in our institution):** the contrast-test-of-least-squares-means IF quantifications (p53-S15, p21), the xenograft **lme4** LRT (`vivo_xenograft`), and **fgsea** GSEA — these stay linked-external until native implementations or the D53 reproducible-external-execution path lands (shared with limma).

Net: the wet-lab tier is fetchable and **partly recomputable**, not a dead end. *Update 2026-06-13:* all the Source Data (10 per-figure XLSX) is now vendored + checksummed, and `va_competition` (ED Fig 3b competition assay) **has been moved from linked-external to kernel-recomputed** via the new `NestedAnovaAnalysisPlan` dispatch (increment 8) — `concl_val_recomputed` reproduces the paper's two-way ANOVA exactly (KM12 P=2.7e-19, OVK18 P=1.2e-7). The cell-cycle (ED 4b) and apoptosis (ED 4c,d) readouts use the same `value ~ is_WRN + guide` design and dispatch, so they're now a mechanical follow-up (data + capability both in hand). The IF lsmeans contrasts, xenograft lme4, and GSEA remain the external-tool frontier.

---

## 9. Decisions (resolved 2026-06-12)

1. **Data vendoring vs. linking** → **Fetch the minimal Phase-1 slices, content-address (checksum) them, link the rest** by accession/DOI. The vendored slices are pinned Observed provenance; recomputation runs against them.
2. **Recompute fidelity bar** → **Within tolerance, per quantity class — formalised in §5.1.** Binding check = preservation of the qualitative claim; numeric agreement is class-dependent (exact for deterministic; ≤2% relative for effect sizes; ~1 order of magnitude on log-scale for P/Q; widened for upstream-linked; ≤5% for permutation). A discrepancy is a recorded *finding*, not a silent pass.
3. **Granularity of Observed wet-lab nodes** → **Per assay-panel, carrying replication metadata**; per-measurement only where a statistical test consumes the individual values.
4. **`onco` module scope** → **Thin-but-reusable.** The dependency / MSI / MMR / TP53 nouns generalise across DepMap-style papers; author them as the reusable `onco` base, keep paper-specific predicates + rules in `wrn-vocab`.
5. **Limma / mixed-models / GSEA as institutions** → **Phase 1 uses only the existing statistics institution** (partial recompute of `D-DIFF`: mean-difference + BH + ranking). A new **Phase 2.5** (§8) builds the **limma moderated-*t*** as an institution to make `D-DIFF` and Phase-4 RNA-seq *fully* recomputed — preferred path: native Julia (`EBayes.jl` + `GLM.jl` + `HypothesisTests.jl`) on the existing D27 substrate, validated against wrapped-R limma as a calibration oracle.
6. **Ref 29** → **Resolved:** Behan, Iorio, Picco et al., *Nature* **568**:511–516 (2019), doi:10.1038/s41586-019-1103-9 (updated in §3).

**Stage-1 is complete.** No blocking open decisions remain; Stage 2 can begin with Phase 0 (fetch minimal slices + author `onco` / `wrn-vocab` / `datasets`) then Phase 1.

---

## 10. Relationship to the broader direction

This encoding is the concrete instance of the open-ended-reasoning pivot (replacing the spec-adherence SAB tasks). It directly exercises:
- the **four-warrant** structure on real science (Observed data, Derived analyses, Declared definitions/rules, the recomputed-checkable tier);
- the **statistics institution** as the engine that turns asserted statistics into *recomputed* warrants (the audit chain closing to raw data — the manifesto claim that spec-adherence tasks could not demonstrate);
- the **reference→claim** discipline (every imported claim is a cited, typed warrant);
- the **typed-tool-boundary** for upstream pipelines (CERES/DEMETER2/GSEA) we link rather than reimplement.

Once Phase 1 is encoded and recomputes cleanly, it becomes both a worked example for the publication *and* the template for encoding further papers — the generalized, machine-checkable form of "make the AI show its work," applied to the human scientific record.
