# What the WRN encoding's predicates actually mean

Compared `experiments/publications/wrn-helicase/chain/*.esl` against the paper's own text,
extracted to `references/publications/WRN-Helicase-Nature-OCR/letter-body.txt` (2,738 words, the
Letter body; reproduced by `extract-letter-body.py` beside it).

## The encoding has three layers, and only the bottom one is the problem

**The certificate algebra is sound.** `wrn:concl_helicase_required` composes:

```
SEP    = litclaim:WRNActivitiesSeparable("WRN")
FR     = onco:FailsToRescue("WRN_cDNA_K577M", "sgWRN_EIJ")
RA     = onco:RequiresActivity("WRN", "helicase")
RULE_P = SEP -> FR -> RA
app(app(declared(RULE, RULE_P), declared(LIT, SEP)), declared(ART, FR))
```

That is modus ponens twice over a declared bridge, with each premise carrying its own ground. The
structure is real and checkable.

**The bridge rules are honest.** `RULE_P` is a Declared implication — an inference the author
asserts and signs for. The design says an implication can only enter as a ground, and it does.

**The atoms are not propositions.** `onco:RequiresActivity("WRN", "helicase")` is an opaque
constructor applied to two bare strings. Nothing in it says WRN is a gene, that "helicase" names
an enzymatic activity, or what "requires" asserts. Three separate defects:

1. **The meaning is in the name, or in a comment, or nowhere.** Of 34 `onco:` predicates, **11
   carry no gloss at all** — including `onco:SyntheticLethal`, the paper's central claim:
   `onco:MSI`, `MSS`, `MMRloss`, `TP53intact`, `TopDifferentialDependency`, `SelectivelyEssential`,
   `DependencyCorrelatesWithMutatorLoad`, `StrongBiomarker`, `OnlyMSISelectiveInFamily`,
   `SyntheticLethal`, `RequiresActivity`. The other 23 have a `//` gloss the kernel cannot read.

2. **Arguments are `core:string`, not entities.** Every binary predicate is
   `core:string -> core:string -> Prop`. `"WRN"` is not the gene resource; `"MSI"` is not the
   phenotype. Two occurrences of `"MSI"` in different predicates are the same string by accident,
   not the same thing by reference — and `"MSI"` is used as a *context* in
   `SelectivelyEssential("WRN","MSI")` and as a *biomarker* in `StrongBiomarker("MSI","WRN_dependency")`.

3. **Argument slots are untyped and inconsistent.** The second slot holds a context (`"MSI"`), an
   activity (`"helicase"`), an assay (`"xenograft_growth"`), a cell line (`"KM12"`), a dataset
   (`"Achilles_MSI"`), a gene family (`"RecQ_helicases"`) and a gene (`"WRN"`, in
   `ModulatesDependence("TP53","WRN")`). Nothing constrains which.

## What each core predicate is standing in for

| encoded atom | the paper's sentence | what it actually asserts |
|---|---|---|
| `SyntheticLethal("WRN","MSI")` | "These findings show that WRN is a synthetic lethal vulnerability and promising drug target for MSI cancers." | a *summary judgement* over the whole study, not a measured result |
| `SelectivelyEssential("WRN","MSI")` | "…the RecQ DNA helicase WRN was selectively essential in MSI models in vitro and in vivo, yet dispensable in models of cancers that are microsatellite stable." | a two-sided contrast: essential in MSI **and** dispensable in MSS. The encoding drops the second half |
| `TopDifferentialDependency("WRN","Achilles_MSI")` | "Projects Achilles … and DRIVE each independently identified WRN … as the top preferential dependency in MSI compared to MSS cell lines (Q = 4.8 × 10⁻²⁴ and 1.5 × 10⁻⁶, respectively)" | a *rank* within a named screen, with a q-value. Rank and statistic are both absent from the atom |
| `RequiresActivity("WRN","helicase")` | "By contrast, helicase inactivation prevented rescue (Fig. 2b, c), indicating that the helicase domain is a candidate therapeutic target." | an inference from a failed rescue, hedged ("indicating", "candidate") |
| `DispensableActivity("WRN","exonuclease")` | "Exonuclease inactivation did not attenuate rescue, suggesting this function is dispensable." | hedged ("suggesting"); the encoded atom is flat |
| `OnlyMSISelectiveInFamily("WRN","RecQ_helicases")` | "By contrast, none of the four other RecQ DNA helicases were preferentially essential in MSI cell lines." | a *negative universal* over a named family of four. The atom has no quantifier |
| `CausesDSBs("WRN","MSI")` | "WRN silencing in MSI, but not MSS, cells substantially increased … 53BP1 foci, which are markers of double-stranded DNA breaks" | again two-sided, and mediated by a *marker* — the DSB is inferred from foci |
| `ModulatesDependence("TP53","WRN")` | "p53-intact MSI cell lines (n = 23) were more sensitive to WRN loss than their p53-impaired (n = 13) counterparts (P = 0.02, Wilcoxon rank-sum test)" | a comparison between two stratified groups with sizes and a test |
| `NotViaTelomereDefect("WRN","MSI")` | "First, we investigated whether a telomere defect precipitates the synthetic lethal relationship…" | a *rejected hypothesis* — a negative result |
| `ContributesToDependence("dMMR","WRN")` | "These data suggest that MMR deficiency alone contributes to the synthetic lethal interaction that we found, although it does not fully explain this interaction" | explicitly partial: contributes **and** does not fully explain. The encoding keeps only the first clause |

## What the comparison shows

**The atoms lose four things the source sentence carries.**

- **Contrast.** Six of the ten above are two-sided in the paper (MSI *but not* MSS; intact *versus*
  impaired). The encoded atom keeps one side. `SelectivelyEssential("WRN","MSI")` cannot express
  "and dispensable in MSS", which is half of what selectivity means.
- **Hedging.** "suggesting", "indicating", "support that" are the authors' own epistemic markers.
  The encoded atom asserts flatly. This is the substitution the two-stratum design exists to
  prevent, reappearing inside a single atom.
- **Quantification.** "none of the four other RecQ helicases" is a bounded universal.
  `OnlyMSISelectiveInFamily("WRN","RecQ_helicases")` has no quantifier and no family membership.
- **The statistic.** `Q = 4.8 × 10⁻²⁴`, `P = 0.02`, `n = 23`/`n = 13` appear in the source
  sentences and in the recompute plans, but not in the atom that the certificate reasons over.

**And one thing the atoms add that the paper does not have.** `onco:SyntheticLethal("WRN","MSI")`
is a conclusion the authors draw in their final paragraph, not a measurement. It sits in the same
syntactic class as `onco:CausesDSBs("WRN","MSI")`, which is an assay readout. The encoding gives
no way to tell a summary judgement from a measured contrast.

## What replacing them requires

The certificate algebra and the declared bridges stay. Only the atoms change — from a constructor
over strings to a parsed proposition over lexicon-grounded entities.

Three things must be true of the parser output for the swap to be sound:

1. **Entity identity.** "WRN" must resolve to a gene resource and "MSI" to a phenotype, so that
   two atoms mentioning WRN mention the *same* WRN. This is what the glossary and the UMLS/WordNet
   alignment exist to supply, and it is why the encoding is gated on the lexicon snapshot.
2. **The bridge rules must still type.** `RULE_P = SEP -> FR -> RA` is an implication between three
   atoms; replacing them changes the implication's type. The rules were authored against the opaque
   atoms and will have to be re-authored against the parsed ones.
3. **Contrast and hedging need somewhere to live.** A parsed "essential in MSI but not MSS" is one
   proposition with internal structure, not two atoms. Whether the hedge ("suggesting") belongs in
   the proposition or in the ground is a modelling decision this note does not settle — but it is
   the decision that determines whether the replacement is an improvement or just a longer atom.

## Open question this raised

`litclaim:*` (11 predicates) are imported claims from *cited* papers, not assertions of this one.
They are opaque in the same way, but their source text is a different document, so the parser
pipeline cannot reach them from this paper's body. Either they stay Declared with a prose gloss —
which is defensible, since a citation is a declaration — or they need their own source text.
