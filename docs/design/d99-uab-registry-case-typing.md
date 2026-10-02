# D99 — Typing a PMI registry case (UAB experiment 02)

**Status: proposed** (2026-10-01). Nine decisions, none taken. Builds on UAB experiment 01
(`experiments/residue-variant/`, #267) and the HPO import and HPO↔UMLS alignment (4b07e11 on
`uab-experiment-02`). Inputs: `experiments/uab/UAB Round 1/02-synthetic-pmi-registry/` —
`README.md`, `registry-schema.md`, `synthetic-cases.json`, `SYN-26-002-analyst-notes.md`.

## The task

The package asks: "is this the kind of case Eigenius can work, and what would you need in hand to
start?" (`README.md:11-12`). The check it sets is the analyst note's decomposition of SYN-26-002:
seven claims, each assigned Observed, Declared, Ill-typed or "Declared bridge"
(`SYN-26-002-analyst-notes.md:78-86`), and four items the analyst could not type (`:117-128`).

| Case | Shape | Variant rows | HP terms |
|---|---|---|---|
| SYN-26-001 | undiagnosed, three VUS | 3 (one protein-only) | 12 |
| SYN-26-002 | hemizygous, Likely Pathogenic, failed therapy | 1 | 6 |
| SYN-25-003 | compound heterozygous, closed with an outcome | 2 | 5 |

Experiment 02 delivers:

1. the three cases converted to a chain layer by a converter, without the parser;
2. SYN-26-002's seven claims and four untyped items as chain resources, each with its grounds;
   claim 6 is experiment 01 restated in the general variant model;
3. for each Declared ground, what UAB would have to supply for it to become Observed.

**In place.**
- **HPO 2026-09-01 on the chain.** 20,533 resources, with every root `subclass_of lexicon:Entity`. 54,595 UMLS lexical entries denote 18,419 HP classes.
- **Grounds.** `justification:Grounds` has the constructors `declared`, `observed`, `verified`, `app`, `sum_l`/`sum_r` and `instantiate`.
  - `prov:DeclarationTrace` requires an agent and a timestamp, and admits `IsDeclaredAs`.
  - `prov:ObservationTrace` requires the generating `prov:Activity` and a timestamp, and admits `IsObservedAs`.
  - `justification:Declaration` requires a proposition and an agent.
- **Sources.** `reference:Reference` and `reference:Citation` exist, plus `prov:Source` for a source the chain does not hold.
- **Discourse kinds** (a closed set): `enc:Finding`, `Observation`, `Classification`, `Hypothesis`, `Suggestion` and `Assertion`.
- **Logic.** ESL has `exists` (Sigma), `logic:And` and `logic:Or`.

**Missing.**
- No vocabulary exists for variants, gene products, persons, cases, phenotype annotations, treatments, questions or recommendations.
- `ncbi:Gene` carries HGNC only inside its dbXrefs string.
- SO and GENO are not loaded.
- ClinVar is not imported.
- The WordNet import reads hypernym and instance edges only (`crates/eigenius-wordnet/src/wndb.rs`), so part holonyms are not on the chain.

## Decisions

| # | Decision | Proposal |
|---|---|---|
| 1 | Variant resolutions | `NucleotideAllele`, `ProteinAllele`, `Location`, related by two non-injective maps (VRS / Cat-VRS) |
| 2 | A variant known only at protein resolution | `Carries` takes a nucleotide allele. A protein-only row is an existential over the alleles that translate to its protein change |
| 3 | Where functional predicates sit | On `ProteinAllele`. A declared per-allele bridge carries them to a nucleotide allele |
| 4 | Allele identity | IRI minted from (reference, interval, alternate state). The VRS digest is recorded where a sequence exists |
| 5 | Grounds of a registry value | Declared by the registry, with the tier-2/3 record as primary source. Observed only with tier 3 in hand |
| 6 | Case shape | Phenopackets v2 as the shape reference. Zygosity, ACMG tier, impact and consent each typed separately |
| 7 | Schema quirks | Per quirk, below |
| 8 | The four untypable items | Goal → questions. ClinVar → observation of a release. No re-trial → recommendation resting on a conclusion. Trial 1 → the physician's Declared summary |
| 9 | Vocabulary placement | Chain-loaded layers, not bootstrap |

### 1. Variant resolutions

A registry row gives a variant at up to three resolutions:
- `RefSeq` + `Genotype`, a c. change. The two joined form `HGVS Notation`; all five rows that carry both agree.
- `Protein Sequence Change`.

Neither nucleotide field is required. The literature adds a coarser resolution, the residue ("Leu44").

| Resolution | SYN-26-002 | VRS / Cat-VRS 1.1 |
|---|---|---|
| nucleotide allele | `NM_000010.7:c.131T>C` | VRS `Allele` on a transcript `SequenceReference` |
| protein allele | `p.Leu44Pro` | VRS `Allele` on a protein reference. The alleles that translate to it form a `ProteinSequenceConsequence`: one `DefiningAlleleConstraint`, relation `translation_of` |
| location | `Leu44` | VRS `SequenceLocation`. The alleles at it form a `CategoricalVariant` with a `DefiningLocationConstraint`; Cat-VRS's own example is `BRAF V600` |

Two maps connect the resolutions, and neither is injective:
- `TranslatesTo`: c.130C>T ↦ p.Leu44Phe and c.131T>C ↦ p.Leu44Pro. Several codons encode one amino acid.
- `At`: p.Leu44Phe ↦ Leu44 and p.Leu44Pro ↦ Leu44.

Proposed (sketch):

```esl
namespace variant = "urn:eigenius:variant";

class variant:SequenceReference : lexicon:Entity { … }   // a transcript (NM_…), or a gene's protein product
class variant:Location : lexicon:Entity { … }            // reference + interval + reference residue(s)
class variant:Allele : lexicon:Entity { … }              // location + alternate state
class variant:NucleotideAllele : variant:Allele { … }
class variant:ProteinAllele : variant:Allele { … }
class variant:CategoricalVariant : lexicon:Entity { … }  // one defining constraint (Cat-VRS)

data variant:TranslatesTo : variant:NucleotideAllele -> variant:ProteinAllele -> Prop {}
data variant:At : variant:Allele -> variant:Location -> Prop {}
data variant:MemberOf : variant:Allele -> variant:CategoricalVariant -> Prop {}
```

Experiment 01's classes map onto this model:
- `avpr2:Variant` is a `NucleotideAllele`.
- `avpr2:Residue` is a `Location`.
- `avpr2:HasResidue` is `TranslatesTo` composed with `At`.

The registry carries no protein accession. A `ProteinAllele`'s reference is therefore "the protein product of gene G", with the isoform unspecified.

### 2. A variant known only at protein resolution

SYN-26-001's SYNSYN3 row holds `p.Gly88Ser` and no nucleotide change. Its Notes say: "nucleotide
change and RefSeq were not stated in the lab report text the analyst worked from". The analyst
asks whether a subject type requiring nucleotide resolution makes this row unwritable (`:109-113`).

A person carries nucleotide sequence, so `Carries` takes a `NucleotideAllele`. The row is written
at the resolution it has, as an existential:

```esl
exists a : variant:NucleotideAllele =>
    logic:And(case:Carries(syn26_001:proband, a, case:heterozygous),
              variant:TranslatesTo(a, synsyn3:p_Gly88Ser))
```

What follows from that:
- A claim about a named nucleotide allele of SYNSYN3 does not apply to the row: the existential names no instance.
- A claim about `p.Gly88Ser` applies through decision 3's bridge.
- The row can be written, and the kernel refuses exactly the substitutions that need a finer resolution than the row has.

**Not proposed: `Carries : Individual -> CategoricalVariant -> Prop`.** This is Cat-VRS's approach, with the category as the subject. It puts two subject types on one relation, which is the residue/variant collapse inside the vocabulary. Categorical variants still exist, but as the subjects of literature claims made at category resolution.

### 3. Where functional and drug-response predicates sit

Ligand binding, membrane trafficking and response to a receptor agonist are properties of the
receptor protein. Proposed: they take a `ProteinAllele`. This follows the 2026-09-30 ruling that
essentiality, dependency and drug-target claims denote the protein.

Applying a protein-level claim to a patient's nucleotide allele takes two things:
- `TranslatesTo`;
- a per-allele Declaration that the allele acts through its protein consequence, `variant:ProteinMediated(a)`. This excludes splice and NMD effects.

SYN-25-003's `c.1105del` (a frameshift with predicted NMD) has no true `ProteinMediated` declaration, so no protein-level claim transfers to it.

Claim 6 under this model:

```esl
// Declared, attributed to the survey's authors (the citation is synthetic)
exists q : variant:ProteinAllele =>
    logic:And(variant:At(q, syngene:Leu44), drug:Responsive(q))
```

Concluding `drug:Responsive(syngene:p_Leu44Pro)` from this claim fails:
- `justification:instantiate` needs `Grounds(forall q => At(q, Leu44) -> Responsive(q))`.
- The survey grounds an existential, and no `Grounds` constructor eliminates one.
- The remaining route is a declared implication from the survey claim to the named instance. That is experiment 01's run D, which states the substitution as a bridge and attributes it.

Experiment 01's refusals are this refusal with the survey's subject typed as a residue:
- A, the class mismatch;
- B, no `IsDeclaredAs` witness.

**Alternative: predicates on `NucleotideAllele`**, which was experiment 01's choice. Assay results and protein-level reports would then be typed one level finer than their evidence. Each would need a nucleotide subject that its source does not give.

### 4. Allele identity

VRS identifies an allele by a digest of its normalized form (`ga4gh:VA.…`), and normalization needs
the reference sequence. The synthetic references (`NM_000010.7` and the others) have no sequence.

Proposed:
- **IRI.** It is minted from the structured fields (reference accession, interval, alternate state). Two spellings of one allele on one reference then get one IRI.
- **VRS digest.** It is recorded as a property where a sequence is available.
- **One allele on two references** (transcript and genome, or two transcripts) gets two IRIs. They are related by a declared or computed congruence, Cat-VRS `CanonicalAllele`'s `liftover_to` / `transcribed_to`. This is out of scope for the synthetic set, which has one transcript per gene.

### 5. The grounds of a registry value

The registry is tier 1. It transcribes tier 3 (clinic notes and lab reports in UAB ShareFile) and
tier 2 (analyst notes in Drive) (`README.md:14-31`). The analyst note's first row says the same: the
variant call is Observed in the "Panel report PDF in ShareFile. The registry row is a transcription
of it."

Proposed:
- Every registry value is a `justification:Declaration` attributed to the PMI registry, a `prov:Organization`.
- Its `prov:had_primary_source` is a `prov:Source` stub for the tier-2 or tier-3 record it transcribes.
- No tier-1 value is Observed.
- With a tier-3 record in hand, an `ObservationTrace` adds an `observed` ground to the same proposition. The trace names the generating activity, such as the panel sequencing run or the urine-volume measurement. The Declaration stays, and `sum_l`/`sum_r` combine the two grounds.

### 6. The case shape

GA4GH Phenopackets v2 is the shape reference for cases, as VRS is for variants. The elements used:
- `Individual`: an imprecise date of birth, and `sex`, `karyotypic_sex` and `gender` as three separate fields.
- `PhenotypicFeature`: an HP class, plus `excluded`, `onset` and `evidence`.
- `VariantInterpretation`: an ACMG classification over a `VariationDescriptor` that carries `allelic_state`.
- `MedicalAction`: a `Treatment`, with `response_to_treatment` and `treatment_termination_reason`.

Proposed: `case` classes, one per element the registry fills.

| Registry field | Typed as | Ground / note |
|---|---|---|
| `Case ID` | `case:case_id` on the record | an identifier, not the identity (§7) |
| `Year of Birth` | `case:birth_year` on the Individual | Declared |
| `Age Calculation`, `Age` | not stored; computed for a stated date (§7) | — |
| `Gender` | `case:sex` on the Individual | Declared; see §7 for `Males`, `Females`, `Family` |
| `Participant Location` | a region resource (§7) | Declared |
| `Ethnicity` | an OMB category | Declared |
| `Diagnosis`, `Symptoms`, `Case History`, `Medications`, `Case Review Next Steps`, `Outcome (Details)` | `enc:DiscourseUnit` text; no proposition until parsed | Declared |
| `Participant/Family Goal` | questions (§8a) | Declared |
| HP Terms | a phenotypic feature: `case:HasPhenotype(individual, <HP class>)` | Declared; label checked (§7) |
| Gene Info row | `case:Carries` (6a), `case:ClassifiedAs` (6b), `variant:Impact` (6c) | Declared |
| `Confirmed de novo` | `true` → `case:DeNovo(a, individual)`; `false` → nothing (§7) | Declared |
| `Status`, `New Status Tags`, `Analyst Case Status`, `Case Category`, `Case Origin` | workflow state, with its timestamp | Declared |
| timestamps | `prov:timestamp` on the state change | Declared |
| consent fields | gate the load (6d) | Declared |
| `Outcome`, `Research Report Sent`, `Action Was Taken on Research Report` | outcome records | Declared |
| `Medical Records`, `Case Notes`, `Case Presentation` | `prov:Source` stubs; the URLs are not loaded | — |

**6a. Zygosity.** `Zygosity` mixes three axes:
- allelic state: `Homozygous`, `Heterozygous`, `Hemizygous`;
- a relation between two alleles: `Compound Heterozygous`, recorded on each of SYN-25-003's two rows;
- tissue distribution: `Mosaic`.

Proposed:
- `case:Carries(individual, allele, state)`, with three state values.
- `Compound Heterozygous` becomes `heterozygous` on each allele plus a Declared `case:InTrans(a1, a2)`. SYN-25-003's Notes name a paternal allele and a maternal allele, which is what places the two in trans.
- `Mosaic` becomes a separate property.
- The state values are minted locally and aligned to GENO, Phenopackets' `allelic_state` vocabulary, once GENO is loaded. GENO is published as OWL only and needs conversion to OBO Graphs JSON for the OBO importer.

**6b. ACMG classification.** ACMG/AMP classifies a variant for a condition. The registry records the
tier, but not the condition or the classifying lab. Proposed:
- The predicate is `case:ClassifiedAs(allele, tier)`.
- It is declared by the reporting lab where a record names one. For SYN-26-002, the analyst notes say "Panel report classifies Likely Pathogenic". Otherwise it is declared by the registry.
- No pathogenicity proposition `Pathogenic(allele, condition)` is formed, because the condition is not recorded.
- The tier is a closed five-value class, and VUS is one of its values.

**6c. Variant impact.** Four of the five values are Muller's morphs (hyper-, hypo-, anti-, amorphic);
the fifth is `Unknown`. Proposed:
- `variant:Impact : variant:Allele -> variant:Morph -> Prop`.
- It is typed on `Allele`, so a protein-only row and a predicted-NMD frameshift can both carry an impact.
- `Unknown` writes nothing.
- Cat-VRS's `FunctionConstraint` names SO for functional consequence, so the values align to SO once SO is loaded.

**6d. Consent.** The package names its practical path as "de-identified data, and a case with both consent boxes
checked" (`README.md:60`). SYN-26-001 and SYN-26-002 have both consents. SYN-25-003 has the study
consent but not genomic data sharing.

Proposed: the converter applies consent as it would to the live base.
- Study consent gates the case.
- Genomic-data-sharing consent gates its Gene Info rows.
- So SYN-25-003 loads without its two variants.

The package does not say whether coded variants (HGVS strings) count as genomic data under that consent.

### 7. Schema quirks

| Quirk | Proposed |
|---|---|
| The UK appears under four labels (England, Great Britain, United Kingdom, Wales); 37 country options for 34 countries | The labels are not synonyms. England and Wales are constituent countries of the UK, and Great Britain excludes Northern Ireland. Each label denotes its own region, and a country count is a query up a part-of relation to sovereign states. The misspelled Philippines is a variant spelling of one region. Needs a part-of relation; WordNet's part holonyms are not imported. |
| Two duplicate Case IDs in ~690 rows | `Case ID` is not the identity. A record's IRI comes from its Airtable record id, and `case:case_id` is a property. A query finds records that share a value; whether two of them are one case is a curator's Declaration. The package inlines records without Airtable ids, so for the synthetic set the converter keys on Case ID (three distinct values). |
| `Age` means current age in the formula and age at intake in some reporting; pediatric fraction 62% vs 71% | Store `Year of Birth` only. Age is computed for a stated date, as an interval of two integers, because month and day are not recorded. Current age and age at intake are two such computations, at today and at `Case Created Timestamp`. Example, SYN-25-003: born 1998, case created 2025-06-03, so 26 or 27 at intake, while `Age Calculation` = 28. A pediatric fraction is a query that names its date. |
| Sparse denominators: ethnicity 210/688, origin 126, category 151 | An empty field or an empty multi-select writes nothing (open world). A rate is a query whose denominator is the set of records where the field is present, and it reports that denominator. |
| Checkbox `false` | Airtable does not distinguish unset from false. SYN-26-002's `Confirmed de novo: false` sits beside "Maternally inherited" in its Notes. Alone, `false` means "not confirmed de novo", which is true of an inherited allele and of an untested one. So `false` writes nothing, and inheritance comes from Notes as a Declared `case:InheritedFrom`. |
| HP ID and label disagree (found against HPO 2026-09-01; history from 17 earlier `hp.obo` releases, 2018-03-08 to 2026-06-23) | SYN-26-002 codes `HP:0004918` with label "Hypernatremic dehydration". HP:0004918 is *Hyperchloremic metabolic acidosis* and *Hypernatremic dehydration* is `HP:0004906` in all 18 releases, so the ID is wrong; the case's Symptoms field says "recurrent hypernatremic dehydration". SYN-25-003's `HP:0003236` label "Elevated circulating creatine kinase concentration" was HPO's label from 2021-06-08 to 2026-02-16 (the case was created 2025-06-03); the 2026-06-06 release renamed it "… activity" and kept no synonym. SYN-26-001's `HP:0100704` "Cortical visual impairment" was HPO's label in 2018-03-08, renamed by 2019-02-12 and kept as a synonym; the case was created 2026-02-11, so HP Terms rows likely keep the label from when the term was first added. `HP:0002353` "Abnormal EEG" is a current synonym and was never the label since 2018. **Proposed:** the converter checks each label against the term's label and synonyms in the loaded release, then against the term's labels in earlier releases. A current synonym passes. A former label passes and is reported as stale. A label of no release of that term is reported, and its annotation is not loaded until someone declares which half is right. HPO drops some former labels (HP:0003236's), so the check needs the release history, not the current synonyms. |
| `RefSeq` present where Notes say none was stated | SYNSYN3 records `NM_000003.2`, but its Notes say the lab report stated no RefSeq. The value has no recorded source. It is transcribed as a Declaration with no primary source, and builds no allele. |
| `Genotype` holds an allele expression | The value is a c. change without its reference. A genotype in the GENO and Phenopackets sense is alleles plus allelic state. The converter checks that `HGVS Notation` = `RefSeq:Genotype`. |
| `Gender` values `Males`, `Females`, `Family` | A case's subject can be several people. `case:subject` takes one or more Individuals, and sex is recorded per Individual. All three synthetic cases have one subject. |

### 8. The four untypable items

**8a. The family's goal.** The goal reads: "Understand why the medication did not work, and whether there is an
alternative. Mother specifically asked whether the published reports the nephrologist relied on
were about the same variant her son has." It holds three questions. A question has answers, not a truth
value, so it takes no grounds.

Proposed: `case:Question` with these properties:
- `case:asked_by`: the mother, a `prov:Person`. The DeclarationTrace records the registry as the party that transcribed the question.
- `case:whether`: for a polar question, its content Prop.
- `case:answered_by`: a `justification:Conclusion` or a `justification:Declaration`.

| Question | Kind | Answer in this chain |
|---|---|---|
| were the reports about her son's variant | polar | "No" needs a positive ground: the survey's cohort carried c.130C>T (Declared; the analyst read it from the survey), and c.130C>T ≠ c.131T>C. The kernel's refusal of the substitution alone answers "not established", not "no". |
| why the medication did not work | why | the mechanism chain of 8c |
| whether there is an alternative | polar | the chaperone hypothesis: `enc:Hypothesis`, resting on the Declared cell-model-to-human bridge (claim 7) |

With this model, `Completed (Goal Met)` (SYN-25-003's status) has a checkable reading: every question of the goal has an answer.
Why-questions have no semantics yet; the polar questions need only `case:whether`.

**8b. Absence from ClinVar.** The registry's Notes say "Absent from ClinVar", with no release and no date. The
analyst notes say c.130C>T is "in ClinVar as pathogenic" and c.131T>C is "absent from ClinVar
entirely".

Proposed:
- **The propositions are about a release, not about the allele:** `clinvar:NotListed(release, allele)` and `clinvar:ListedAs(release, allele, tier)`.
- **A release is a finite artifact, and querying it is an Activity.** The query's result is Observed: an `ObservationTrace` whose `prov:was_generated_by` is the query activity, with `prov:used` the release and `prov:started_at` the time T.
- **Nothing derives a claim about the allele from `NotListed`.** The retrieval failure in the notes ("the absence of ours reads as the absence of a problem") is that derivation, and it would need a declared bridge.
- **In this package it is Declared by the registry.** The synthetic allele cannot be looked up. For a real allele, the converter's companion runs the lookup and writes the trace. That needs a ClinVar release resource, not a ClinVar import.

**8c. The clinical judgment not to re-trial** (`:60-68`).

| Step | Proposition | Ground |
|---|---|---|
| 1 | p.Leu44Pro yields <4% of wild-type binding sites at the membrane | Declared with citation; Observed by the assay's authors. The citation is synthetic. |
| 2 | the patient carries c.131T>C, hemizygous; it translates to p.Leu44Pro and is protein-mediated | Declared (the registry, transcribing the panel report). Translation is computable from a real transcript. |
| 3 | mechanism bridge: with receptor at the membrane below threshold *k* of wild type, the ligand-side agent gives no reduction in urine output at any dose | Declared by the analyst. *k* is not stated in the notes. |
| 4 | the agent will not reduce this patient's urine output at any dose | `justification:Conclusion`: `app` of 3 to 1 and 2 |
| 5 | do not re-trial at a higher dose | `case:Recommendation`, Declared by the analyst; `case:rests_on` step 4 |

The two failed trials ground "no measurable reduction at the doses tried". The recommendation
concerns a higher dose, and only the mechanism bridge reaches that. Step 5 is a directive, so it carries no
grounds of its own; it names the conclusion it rests on.

**8d. Trial 1's outcome.** The trials are `MedicalAction`s with a `Treatment`. Proposed:
- `case:NoMeasurableReduction(trial_1)` is Declared by the treating nephrologist. Its primary source is the clinic note (tier 3, not held), and "measurable" is the physician's threshold.
- Trial 2 has pre/post volumes in tier 3. With those in hand it is Observed; with only tiers 1 and 2 it is Declared.

### 9. Vocabulary placement

- **Location.** `variant` (`ontologies/variant/`) and `case` (`ontologies/case/`) are chain-loaded layers, not bootstrap ontologies. A bootstrap edit moves the manifest and forces a reseed (about 12 minutes) and a gate rerun.
- **Synthetic genes.** They are minted in an experiment namespace (`syngene:`, `synsyn3:`, …) and are never `ncbi:Gene` resources.
- **Real genes.** These resolve by HGNC id to `ncbi:Gene`. That needs the HGNC xref out of the dbXrefs string.

## What UAB would supply

| To make Observed, or to complete | Needs | PHI |
|---|---|---|
| variant call and zygosity | the panel report (tier 3) | yes |
| trial 2 outcome | clinic notes with pre/post volumes (tier 3) | yes |
| trial 1 outcome | volumes, which the notes say do not exist | — |
| ClinVar status | a real allele, and a release to query | no |
| <4% binding, survey cohort alleles | the cited papers, resolved to `reference:Reference` | no |
| a pathogenicity proposition | the condition each ACMG tier is for, and the classifying lab | no |
| record identity in the live base | Airtable record ids | no |

## Implementation order

0. **Reseed.** Reseed this branch from a clean tree, because the OBO meta-ontology bootstrap moved in 4b07e11. Record selections, ranks and `baseline.json` against the new snapshot. This is required before a merge to main and is independent of the steps below.
1. **Layers.** Build the `variant` and `case` layers (decisions 1–4 and 6), with tests that the three resolutions and the existential row check.
2. **Converter.** Convert `synthetic-cases.json` to an ESL layer without the parser. It applies decision 5's grounds, 6d's consent rule and §7's checks (HP label, HGVS consistency), and reports the HP:0004918 row.
3. **SYN-26-002 chain.** Encode the seven claims and the four items, with claim 6 restated per decision 3.
4. **Integration test.** Add an ignored test against the snapshot. It asserts each claim's ground kind and the claim-6 refusal.
5. **README.** Write it in the experiment directory: the analyst table mapped to chain resources, and the supply table.
6. **Optional: parse prose.** Parse `Case History` and Notes prose with the HPO-aligned lexicon, under its own gate.

## Open

- **Decision 3 vs experiment 01.** Should functional predicates sit on the protein allele or on the nucleotide allele?
- **Phenotype-annotation form.** Is an HP class a value, or an existential over its instances? Whichever form the parser gives "the patient has polyuria" keeps registry rows and parsed history in one form.
- **Disequality.** The "no" answer in 8a needs the kernel to prove c.130C>T ≠ c.131T>C, and unification failure is not a proof of disequality.
- **Threshold *k* in 8c.** The notes give <4% for p.Leu44Pro and "wild-type levels" for p.Leu44Phe.
- **The HP:0004918 row.** Report it to UAB, or declare HP:0004906 for the experiment.
- **Consent scope for coded variants** (6d).
