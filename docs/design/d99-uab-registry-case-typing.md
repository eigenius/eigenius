# D99 — Typing a PMI registry case (UAB experiment 02)

**Status: decided** (all eleven decisions), 2026-10-02 (the owner: decision 3 on the protein allele, the rest
as proposed, existentials in the parser's encoding; decision 10 option A, and `DecEq` removed; decision 11
after OWL and SSSOM). Proposed
2026-10-01. Builds on UAB experiment 01
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
- **Logic.** ESL has `logic:And`, `logic:Or`, `logic:False`, and `exists`, which is a Sigma type.
  - Sigma is predicative: it lives at `Sort(max m n)` (`kernel/src/nbe/check/conv.rs`), so `exists a : C => P(a)` over a class `C` is in `Type 1`, not `Prop`. `Grounds : Prop -> Type 2` does not take it.
  - A Prop-valued existential is written in the encoding the parser emits, which is a `Prop` because Pi into `Prop` is impredicative: `forall (R : Prop) => (forall (x : exists x : C => restrictor(x)) => body(eigentt:fst(x)) -> R) -> R`. Experiment 01's `claim_1` has this shape. Below, **∃x:C[restrictor]. body** abbreviates it.

**Missing.**
- No vocabulary exists for variants, gene products, persons, cases, phenotype annotations, treatments, questions or recommendations.
- `ncbi:Gene` carries HGNC only inside its dbXrefs string.
- SO and GENO are not loaded.
- ClinVar is not imported.
- The WordNet import reads hypernym and instance edges only (`crates/eigenius-wordnet/src/wndb.rs`), so part holonyms are not on the chain.

## Decisions

| # | Decision | Status |
|---|---|---|
| 1 | Variant resolutions: `NucleotideAllele`, `ProteinAllele`, `Location`, related by two non-injective maps (VRS / Cat-VRS) | decided 2026-10-02 |
| 2 | A variant known only at protein resolution: `Carries` takes a nucleotide allele; a protein-only row is an existential over the alleles that translate to its protein change | decided 2026-10-02 |
| 3 | Functional predicates sit on `ProteinAllele`; a declared per-allele bridge carries them to a nucleotide allele | decided 2026-10-02 |
| 4 | Allele identity: IRI minted from (reference, interval, alternate state); the VRS digest is recorded where a sequence exists | decided 2026-10-02 |
| 5 | Grounds of a registry value: Declared by the registry, with the tier-2/3 record as primary source; Observed only with tier 3 in hand | decided 2026-10-02 |
| 6 | Case shape: Phenopackets v2 as the shape reference; zygosity, ACMG tier, impact and consent each typed separately | decided 2026-10-02 |
| 7 | Schema quirks: per quirk, below | decided 2026-10-02 |
| 8 | The four untypable items: goal → questions; ClinVar → observation of a release; no re-trial → recommendation resting on a conclusion; trial 1 → the physician's Declared summary | decided 2026-10-02 |
| 9 | Vocabulary placement: chain-loaded layers, not bootstrap | decided 2026-10-02 |
| 10 | Disequality: kernel-checked, by congruence over identity fields and a literal-apartness rule | decided 2026-10-02 |
| 11 | Class equivalence: `core:EquivalentClasses`, OWL's n-ary axiom as a resource; subsumption only; SSSOM-style justification | decided 2026-10-02 |

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

Decided, and built as `ontologies/variant/variant.esl` (2026-10-02):

```esl
class variant:SequenceReference : lexicon:Entity { requires variant:accession; … }  // Transcript, ProteinProduct
class variant:Location : lexicon:Entity { requires variant:reference, variant:start, variant:end; … }
class variant:Allele : lexicon:Entity { requires variant:reference, variant:start, variant:end, variant:alt; … }
class variant:NucleotideAllele : variant:Allele { … }
class variant:ProteinAllele : variant:Allele { … }

data variant:TranslatesTo : variant:NucleotideAllele -> variant:ProteinAllele -> Prop {…}   // declared
def variant:At(a : variant:Allele, l : variant:Location) : Prop =                          // computed
    logic:And(eigentt:Eq(core:string, eigentt:field(a, variant:reference), eigentt:field(l, variant:reference)),
              logic:And(eigentt:Eq(core:integer, eigentt:field(a, variant:start), eigentt:field(l, variant:start)),
                        eigentt:Eq(core:integer, eigentt:field(a, variant:end), eigentt:field(l, variant:end))));
data variant:ProteinMediated : variant:NucleotideAllele -> Prop {…}                       // declared, per allele
```

- **Identity fields are literals.** An allele names its reference by the reference's accession, as VRS names a sequence reference by its `refgetAccession`: `variant:reference = "NM_000010.7"`. Coordinates are HGVS-style, 1-based and inclusive.
- **`At` is a definition over those fields, not a declared relation.** A declared `At` beside the fields could contradict them. It is proved by `refl`: `kernel/tests/d99_variant_layer.rs` proves p.Leu44Pro at Leu44, and refuses it at position 45 with `LitInt(45) ≠ LitInt(44)`.
- **`TranslatesTo` is declared.** It is computable only from a reference's sequence, which no synthetic reference has.
- **The Cat-VRS categorical variant is not built.** A literature claim at category resolution is the existential of §3, over the alleles `At` a location.

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
at the resolution it has, as an existential, ∃a:NucleotideAllele[TranslatesTo(a, p.Gly88Ser)]. Carries(proband, a, heterozygous):

```esl
forall (R : Prop) =>
    (forall (a : exists a : variant:NucleotideAllele =>
                     variant:TranslatesTo(a, synsyn3:p_Gly88Ser)) =>
        clinical:Carries(syn26_001:proband, eigentt:fst(a), clinical:heterozygous) -> R)
    -> R
```

What follows from that:
- A claim about a named nucleotide allele of SYNSYN3 does not apply to the row: the existential names no instance.
- A claim about `p.Gly88Ser` applies through decision 3's bridge.
- The row can be written, and the kernel refuses exactly the substitutions that need a finer resolution than the row has.

**Rejected: `Carries : Individual -> CategoricalVariant -> Prop`.** This is Cat-VRS's approach, with the category as the subject. It puts two subject types on one relation, which is the residue/variant collapse inside the vocabulary. Categorical variants still exist, but as the subjects of literature claims made at category resolution.

### 3. Where functional and drug-response predicates sit

Ligand binding, membrane trafficking and response to a receptor agonist are properties of the
receptor protein. Decided (2026-10-02): they take a `ProteinAllele`. This follows the 2026-09-30
ruling that essentiality, dependency and drug-target claims denote the protein.

Applying a protein-level claim to a patient's nucleotide allele takes two things:
- `TranslatesTo`;
- a per-allele Declaration that the allele acts through its protein consequence, `variant:ProteinMediated(a)`. This excludes splice and NMD effects.

SYN-25-003's `c.1105del` (a frameshift with predicted NMD) has no true `ProteinMediated` declaration, so no protein-level claim transfers to it.

Claim 6 under this model:

```esl
// Declared, attributed to the survey's authors (the citation is synthetic)
// ∃q:ProteinAllele[At(q, Leu44)]. Responsive(q)
forall (R : Prop) =>
    (forall (q : exists q : variant:ProteinAllele => variant:At(q, syngene:Leu44)) =>
        drug:Responsive(eigentt:fst(q)) -> R)
    -> R
```

Concluding `drug:Responsive(syngene:p_Leu44Pro)` from this claim fails:
- Using the claim means instantiating `R`. With `R := Responsive(p_Leu44Pro)`, `app` then needs `Grounds(forall q : Σ[At(q, Leu44)] => Responsive(fst q) -> Responsive(p_Leu44Pro))`: every responsive allele at Leu44 is p.Leu44Pro. No source grounds that.
- `instantiate` is typed `T : Type 1` and admits `T = Prop`: `kernel/tests/d99_variant_layer.rs` instantiates claim 6 at `R := Responsive(p.Leu44Pro)` (settled 2026-10-02). So the refusal is not that the claim cannot be used. It is that its antecedent has no ground.
- The remaining route is a declared implication from the survey claim to the named instance. That is experiment 01's run D, which states the substitution as a bridge and attributes it.

Experiment 01's refusals are this refusal with the survey's subject typed as a residue:
- A, the class mismatch;
- B, no `IsDeclaredAs` witness.

**Rejected (2026-10-02): predicates on `NucleotideAllele`**, which was experiment 01's choice. Assay results and protein-level reports would then be typed one level finer than their evidence. Each would need a nucleotide subject that its source does not give.

### 4. Allele identity

VRS identifies an allele by a digest of its normalized form (`ga4gh:VA.…`), and normalization needs
the reference sequence. The synthetic references (`NM_000010.7` and the others) have no sequence.

Decided:
- **IRI.** It is minted from the structured fields (reference accession, interval, alternate state). Two spellings of one allele on one reference then get one IRI.
- **Reference accession.** A transcript's is its versioned RefSeq accession. A protein product named only by its gene gets one minted from the gene's HGNC id, `<HGNC id>:p` (`HGNC:000010:p`); the registry carries no protein accession.
- **VRS digest.** It is recorded as a property where a sequence is available.
- **One allele on two references** (transcript and genome, or two transcripts) gets two IRIs. They are related by a declared or computed congruence, Cat-VRS `CanonicalAllele`'s `liftover_to` / `transcribed_to`. This is out of scope for the synthetic set, which has one transcript per gene.

### 5. The grounds of a registry value

The registry is tier 1. It transcribes tier 3 (clinic notes and lab reports in UAB ShareFile) and
tier 2 (analyst notes in Drive) (`README.md:14-31`). The analyst note's first row says the same: the
variant call is Observed in the "Panel report PDF in ShareFile. The registry row is a transcription
of it."

Decided:
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

Decided: `clinical` classes, one per element the registry fills (the vocabulary is `clinical`, not `case`, which is an ESL keyword).

| Registry field | Typed as | Ground / note |
|---|---|---|
| `Case ID` | `clinical:case_id` on the record | an identifier, not the identity (§7) |
| `Year of Birth` | `clinical:birth_year` on the Individual | Declared |
| `Age Calculation`, `Age` | not stored; computed for a stated date (§7) | — |
| `Gender` | `clinical:sex` on the Individual | Declared; see §7 for `Males`, `Females`, `Family` |
| `Participant Location` | a region resource (§7) | Declared |
| `Ethnicity` | an OMB category | Declared |
| `Diagnosis`, `Symptoms`, `Case History`, `Medications`, `Case Review Next Steps`, `Outcome (Details)` | `enc:DiscourseUnit` text; no proposition until parsed | Declared |
| `Participant/Family Goal` | questions (§8a) | Declared |
| HP Terms | a phenotypic feature: `clinical:HasPhenotype(individual, <HP class>)` | Declared; label checked (§7) |
| Gene Info row | `clinical:Carries` (6a), `clinical:ClassifiedAs` (6b), `variant:Impact` (6c) | Declared |
| `Confirmed de novo` | `true` → `clinical:DeNovo(a, individual)`; `false` → nothing (§7) | Declared |
| `Status`, `New Status Tags`, `Analyst Case Status`, `Case Category`, `Case Origin` | workflow state, with its timestamp | Declared |
| timestamps | `prov:timestamp` on the state change | Declared |
| consent fields | gate the load (6d) | Declared |
| `Outcome`, `Research Report Sent`, `Action Was Taken on Research Report` | outcome records | Declared |
| `Medical Records`, `Case Notes`, `Case Presentation` | `prov:Source` stubs; the URLs are not loaded | — |

**6a. Zygosity.** `Zygosity` mixes three axes:
- allelic state: `Homozygous`, `Heterozygous`, `Hemizygous`;
- a relation between two alleles: `Compound Heterozygous`, recorded on each of SYN-25-003's two rows;
- tissue distribution: `Mosaic`.

Decided:
- `clinical:Carries(individual, allele, state)`, with three state values.
- `Compound Heterozygous` becomes `heterozygous` on each allele plus a Declared `clinical:InTrans(a1, a2)`. SYN-25-003's Notes name a paternal allele and a maternal allele, which is what places the two in trans.
- `Mosaic` becomes a separate property.
- The state values are minted locally and aligned to GENO, Phenopackets' `allelic_state` vocabulary, once GENO is loaded. GENO is published as OWL only and needs conversion to OBO Graphs JSON for the OBO importer.

**6b. ACMG classification.** ACMG/AMP classifies a variant for a condition. The registry records the
tier, but not the condition or the classifying lab. Decided:
- The predicate is `clinical:ClassifiedAs(allele, tier)`.
- It is declared by the reporting lab where a record names one. For SYN-26-002, the analyst notes say "Panel report classifies Likely Pathogenic". Otherwise it is declared by the registry.
- No pathogenicity proposition `Pathogenic(allele, condition)` is formed, because the condition is not recorded.
- The tier is a closed five-value class, and VUS is one of its values.

**6c. Variant impact.** Four of the five values are Muller's morphs (hyper-, hypo-, anti-, amorphic);
the fifth is `Unknown`. Decided:
- `variant:Impact : variant:Allele -> variant:Morph -> Prop`.
- It is typed on `Allele`, so a protein-only row and a predicted-NMD frameshift can both carry an impact.
- `Unknown` writes nothing.
- Cat-VRS's `FunctionConstraint` names SO for functional consequence, so the values align to SO once SO is loaded.

**6d. Consent.** The package names its practical path as "de-identified data, and a case with both consent boxes
checked" (`README.md:60`). SYN-26-001 and SYN-26-002 have both consents. SYN-25-003 has the study
consent but not genomic data sharing.

Decided: the converter applies consent as it would to the live base.
- Study consent gates the case.
- Genomic-data-sharing consent gates its Gene Info rows.
- So SYN-25-003 loads without its two variants.

The package does not say whether coded variants (HGVS strings) count as genomic data under that consent.

### 7. Schema quirks

| Quirk | Decided |
|---|---|
| The UK appears under four labels (England, Great Britain, United Kingdom, Wales); 37 country options for 34 countries | The labels are not synonyms. England and Wales are constituent countries of the UK, and Great Britain excludes Northern Ireland. Each label denotes its own region, and a country count is a query up a part-of relation to sovereign states. The misspelled Philippines is a variant spelling of one region. Needs a part-of relation; WordNet's part holonyms are not imported. |
| Two duplicate Case IDs in ~690 rows | `Case ID` is not the identity. A record's IRI comes from its Airtable record id, and `clinical:case_id` is a property. A query finds records that share a value; whether two of them are one case is a curator's Declaration. The package inlines records without Airtable ids, so for the synthetic set the converter keys on Case ID (three distinct values). |
| `Age` means current age in the formula and age at intake in some reporting; pediatric fraction 62% vs 71% | Store `Year of Birth` only. Age is computed for a stated date, as an interval of two integers, because month and day are not recorded. Current age and age at intake are two such computations, at today and at `Case Created Timestamp`. Example, SYN-25-003: born 1998, case created 2025-06-03, so 26 or 27 at intake, while `Age Calculation` = 28. A pediatric fraction is a query that names its date. |
| Sparse denominators: ethnicity 210/688, origin 126, category 151 | An empty field or an empty multi-select writes nothing (open world). A rate is a query whose denominator is the set of records where the field is present, and it reports that denominator. |
| Checkbox `false` | Airtable does not distinguish unset from false. SYN-26-002's `Confirmed de novo: false` sits beside "Maternally inherited" in its Notes. Alone, `false` means "not confirmed de novo", which is true of an inherited allele and of an untested one. So `false` writes nothing, and inheritance comes from Notes as a Declared `clinical:InheritedFrom`. |
| HP ID and label disagree (found against HPO 2026-09-01; history from 17 earlier `hp.obo` releases, 2018-03-08 to 2026-06-23) | SYN-26-002 codes `HP:0004918` with label "Hypernatremic dehydration". HP:0004918 is *Hyperchloremic metabolic acidosis* and *Hypernatremic dehydration* is `HP:0004906` in all 18 releases, so the ID is wrong; the case's Symptoms field says "recurrent hypernatremic dehydration". SYN-25-003's `HP:0003236` label "Elevated circulating creatine kinase concentration" was HPO's label from 2021-06-08 to 2026-02-16 (the case was created 2025-06-03); the 2026-06-06 release renamed it "… activity" and kept no synonym. SYN-26-001's `HP:0100704` "Cortical visual impairment" was HPO's label in 2018-03-08, renamed by 2019-02-12 and kept as a synonym; the case was created 2026-02-11, so HP Terms rows likely keep the label from when the term was first added. `HP:0002353` "Abnormal EEG" is a current synonym and was never the label since 2018. **Decided:** the converter checks each label against the term's label and synonyms in the loaded release, then against the term's labels in earlier releases. A current synonym passes. A former label passes and is reported as stale. A label of no release of that term is reported, and its annotation is not loaded until someone declares which half is right. HPO drops some former labels (HP:0003236's), so the check needs the release history, not the current synonyms. |
| `RefSeq` present where Notes say none was stated | SYNSYN3 records `NM_000003.2`, but its Notes say the lab report stated no RefSeq. The value has no recorded source. It is transcribed as a Declaration with no primary source, and builds no allele. |
| `Genotype` holds an allele expression | The value is a c. change without its reference. A genotype in the GENO and Phenopackets sense is alleles plus allelic state. The converter checks that `HGVS Notation` = `RefSeq:Genotype`. |
| `Gender` values `Males`, `Females`, `Family` | A case's subject can be several people. `clinical:subject` takes one or more Individuals, and sex is recorded per Individual. All three synthetic cases have one subject. |

### 8. The four untypable items

**8a. The family's goal.** The goal reads: "Understand why the medication did not work, and whether there is an
alternative. Mother specifically asked whether the published reports the nephrologist relied on
were about the same variant her son has." It holds three questions. A question has answers, not a truth
value, so it takes no grounds.

Decided: `clinical:Question` with these properties:
- `clinical:asked_by`: the mother, a `prov:Person`. The DeclarationTrace records the registry as the party that transcribed the question.
- `clinical:whether`: for a polar question, its content Prop.
- `clinical:answered_by`: a `justification:Conclusion` or a `justification:Declaration`.

| Question | Kind | Answer in this chain |
|---|---|---|
| were the reports about her son's variant | polar | "No" needs a positive ground: the survey's cohort carried c.130C>T (Declared; the analyst read it from the survey), and c.130C>T ≠ c.131T>C (Verified, decision 10). The kernel's refusal of the substitution alone answers "not established", not "no". |
| why the medication did not work | why | the mechanism chain of 8c |
| whether there is an alternative | polar | the chaperone hypothesis: `enc:Hypothesis`, resting on the Declared cell-model-to-human bridge (claim 7) |

With this model, `Completed (Goal Met)` (SYN-25-003's status) has a checkable reading: every question of the goal has an answer.
Why-questions have no semantics yet; the polar questions need only `clinical:whether`.

**8b. Absence from ClinVar.** The registry's Notes say "Absent from ClinVar", with no release and no date. The
analyst notes say c.130C>T is "in ClinVar as pathogenic" and c.131T>C is "absent from ClinVar
entirely".

Decided:
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
| 5 | do not re-trial at a higher dose | `clinical:Recommendation`, Declared by the analyst; `clinical:rests_on` step 4 |

The two failed trials ground "no measurable reduction at the doses tried". The recommendation
concerns a higher dose, and only the mechanism bridge reaches that. Step 5 is a directive, so it carries no
grounds of its own; it names the conclusion it rests on.

**8d. Trial 1's outcome.** The trials are `MedicalAction`s with a `Treatment`. Decided:
- `clinical:NoMeasurableReduction(trial_1)` is Declared by the treating nephrologist. Its primary source is the clinic note (tier 3, not held), and "measurable" is the physician's threshold.
- Trial 2 has pre/post volumes in tier 3. With those in hand it is Observed; with only tiers 1 and 2 it is Declared.

### 9. Vocabulary placement

- **Location.** `variant` (`ontologies/variant/`) and `clinical` (`ontologies/clinical/`) are chain-loaded layers, not bootstrap ontologies. A bootstrap edit moves the manifest and forces a reseed (about 12 minutes) and a gate rerun.
- **Synthetic genes.** They are minted in an experiment namespace (`syngene:`, `synsyn3:`, …) and are never `ncbi:Gene` resources.
- **Real genes.** These resolve by HGNC id to `ncbi:Gene`. That needs the HGNC xref out of the dbXrefs string.

### 10. Disequality

The "no" in 8a rests on c.130C>T ≠ c.131T>C. The owner (2026-10-02): kernel-checked, designed before
the case chain is built.

What the kernel had (2026-10-02):
- `Id(A, x, y)` in the D47 codec (the `Id` constructor of `eigentt:Term`, declared in `core-ontology.json`). Not in ESL: the bare name `Id` resolves to that quoted-term constructor, so `Id(core:integer, 1, 2)` compiled to a term VALUE, not the type. The ESL guide documented the broken form.
- `Refl`, `IdJ` (J), `DecEq` and `PropAccess` in `Exp` (`kernel/src/nbe/term.rs`), with no codec constructor. No chain-resident term could eliminate an identity, so none could prove `Id(A, x, y) -> logic:False`.
- `DecEq(A, x, y)`, which reduces to `refl` when x and y are equal ground values and to a stuck neutral otherwise. It cannot witness x ≠ y.
- A J rule that ignored its motive and typed `J(A, C, d, x, y, p)` as `C(x, x, refl x)` — what `d(x)` already inhabits — so congruence was not expressible. A blocked J evaluated to `p` applied to `()`.
- `Verified` admitted only from a `justification:Conclusion` carrying a `justification:proof_judgement`. A Conclusion requires a `grounds_judgement`, and a proved proposition has no grounds but itself, so the configuration was unreachable. D89 §2 had decided otherwise ("the proof judgement is a warrant fact available on any proposition-bearing resource") and noted the configuration "designed and empty".
- `PropAccess(e, p)`, which types when `e` has a class type that declares `p` (`find_record_field`), and evaluates on a named resource, since a named individual decodes with its content.

**Decided (A, 2026-10-02) and built: congruence over identity fields, plus literal apartness.**
- **`Apart`.** A new kernel rule, `Apart(A, x, y) : Id(A, x, y) -> logic:False`. It type-checks only when x and y evaluate to distinct canonical literals: string, integer, boolean, rational or unit. Literals are canonical (D93 and D94 made rationals and units so), so distinct literals are distinct values. Floats are refused (`NaN`, `-0.0`), and so are names: two IRIs may denote one thing.
- **J.** It checks its motive `C : (x y : A) -> Id(A, x, y) -> Sort` and its method against `(z : A) -> C(z, z, refl z)`, and types as `C(x, y, p)`. A blocked J is a neutral that reads back to itself.
- **Codec.** `eigentt:Term` gains `Refl`, `IdJ`, `Apart` and `PropAccess`. `subst.rs`'s fragment follows the codec, and gains them plus the `LitRat` and `LitUnit` leaves it lacked.
- **ESL.** `eigentt:Eq(A, x, y)`, `eigentt:refl(x)`, `eigentt:J(A, C, d, x, y, p)`, `eigentt:apart(A, x, y)` and `eigentt:field(e, p)`. Arguments past a form's arity are applied to it, since ESL has no `f(a)(b)`. The printer emits the same forms, and the round-trip corpus covers them.
- **Where the proof rides (D89 §2, implemented).** `justification:proof_judgement` has no domain, and `Verified` is admitted from any resource carrying one. The disequality is a `justification:Declaration` with its proposition, its author and its proof: a manually authored claim with a checked proof.
- **The proof** (`kernel/tests/fixtures/disequality_by_field.esl`):

  ```esl
  fun (p : eigentt:Eq(fx:Allele, fx:c130, fx:c131)) =>
      eigentt:apart(core:integer, 130, 131,
          eigentt:J(fx:Allele,
              fun (x : fx:Allele, y : fx:Allele, q : eigentt:Eq(fx:Allele, x, y)) =>
                  eigentt:Eq(core:integer, eigentt:field(x, fx:start), eigentt:field(y, fx:start)),
              fun (z : fx:Allele) => eigentt:refl(eigentt:field(z, fx:start)),
              fx:c130, fx:c131, p))
  ```

  `kernel/tests/disequality_by_field.rs` pins it:
  - it proves the two alleles distinct by `start` and by `alt`;
  - it grounds a citing conclusion through `Grounds.verified`;
  - a citation of a resource without a proof is refused;
  - an allele and a twin that agrees on every field are not provably distinct;
  - two names are not apart;
  - an apartness about the wrong literals does not connect.
- **Alleles.** Alleles carry the fields decision 4 mints their IRI from as single-valued literals: `variant:reference` (the reference's accession) and `variant:alt` (string), `variant:start` and `variant:end` (integer).
- **Coverage.** Any differing literal field gives a proof. Two minted alleles that agree on every field are one allele by decision 4.
- **Distinctness is of representations.** An allele is a representation on one reference (decision 4), so `Id` on alleles is identity of representations. Distinct `start`s prove two representations distinct. Whether two alleles on different references are the same biological variant is the congruence relation of decision 4, not `Id`. 8a compares two alleles on one reference, `NM_000010.7`.
- **No unique names.** Two resources whose fields agree are never proved distinct, and neither are resources of a class without literal fields. The four UK labels (§7), or a WordNet and a UMLS concept for one thing, stay free to co-denote.
- **`At` is computed, not declared** (§1).
- **Witness keys hash the normal form.** A definition unfolded over a named resource (`variant:At(q, syn:Leu44)`) decodes to `field(syn:Leu44, start)`, which evaluation reduces to `44`. The emit side hashed the decoded term, on D66's invariant that a decoded term is normal, and the check side hashes the evaluated one, so claim 6's Declared witness missed its citation. `witness_admission.rs` now evaluates and reads back before hashing (`hash_normal_proposition`), the check side's procedure, so the two ends agree by construction.

Rejected alternatives:
- **(B) A keyed-class rule.** A class names its key properties; the kernel admits two instances as distinct when their key literals differ, without J. This is A's congruence argument special-cased as a vocabulary feature.
- **(C) Lean.** Export the alleles as a Lean structure with `DecidableEq`, prove by `decide`, and import a `VerificationTrace` (`proof_system = lean4`). This adds Lean to the trusted base for a fact the kernel can check, and the alleles must be exported as definitions, not opaque constants.

**Found on the way, not fixed: a resource-typed field has no consistent value.** `eigentt:field(r, p)` for a `core:resource` property evaluates to `EigonClass(iri)` when the evaluation has a layer, and to `LitString(iri)` when it has none (`nbe/eval/marshal.rs`, `string_role_of`). It is never the referenced individual, which a named individual in a term decodes to (`EigonResource`). A field naming an individual then fails to re-check at its class: `instantiate`'s implicit `P` did, with `EigonPrimitive(String) ≠ EigonClass(variant:SequenceReference)`. The variant layer avoids it by naming references by literal accession. The fix is the referenced resource's own value, and it touches how programs marshal a `ResourceVal` back (embedded, not by IRI). A separate decision.

**Found on the way.** `DecEq`'s typing rule (the `Exp::DecEq` arm of `check_infer` in `kernel/src/nbe/check/mod.rs`) gave `DecEq(A, x, y)` the type `Id(A, x, y)` without checking x ≡ y, so `DecEq(core:integer, 130, 131)` type-checked as a proof of `130 = 131` that evaluated to a stuck neutral. No ESL surface or codec constructor produced it; only the kernel's own tests did. Decided and done (2026-10-02): `DecEq` is removed, and `Apart` takes its place in `Exp`. A checked `DecEq` would only be `refl` under another name.

### 11. Class equivalence

A registry row codes a phenotype in HPO (HP:0000103), and the parser reads "polyuria" as WordNet's
synset (`wn:n14114365`). For the 942 UMLS concepts the WordNet↔UMLS alignment took, the two classes
name one thing and nothing on the chain says so. Rejected:
- **Mutual `subclass_of`.** A class's `subclass_of` is part of its definition: its record type is
  built by walking its parents. A cycle between two classes is a circular definition, which
  declaration ordering refuses (D76 §6.3). Splitting the two edges across layers passes the
  per-layer check and keeps the cycle.
- **Repointing the lexicon at HPO.** The 942 include fever, pain, coughing and headache, so general
  prose would read them as HPO phenotypes. WordNet's own entries would still name the synsets,
  giving every such word two readings.

Decided (2026-10-02):
- **`core:EquivalentClasses`, after OWL 2's `EquivalentClasses(C₁ … Cₙ)`.** An instance lists
  classes that denote the same thing in `core:classes` (a `resource_array` of classes). It is an
  instance, not a declaration: it names classes already declared, and none of them refers to it, so
  the declaration-order graph never sees it.
- **Subsumption only.** `Layer::is_subclass_of`, and so type inhabitation and the query engine's
  subclass closure, treat any two classes listed together as subclasses of each other. Record types
  and property inheritance (`resolve_class_type`, `declared_properties`) never consult it; if they
  did, the mutual-`subclass_of` cycle would be back under another name.
- **Equivalent classes declare the same required properties**, checked at commit. Otherwise an
  instance of one would inhabit the other without its obligations. HPO and WordNet classes require
  nothing.
- **The justification is recorded SSSOM-style.** `core:mapping_justification` names a SEMAPV
  process: here `semapv:MappingChaining`, since each equivalence chains two published mappings. The
  HP code → CUI link is NLM's (MRCONSO, `SAB=HPO`); the CUI → synset link is the WordNet↔UMLS
  adjudication (`experiments/lexicon-align/merges.json`). Attribution and source are `prov:`
  properties. The layer exports to SSSOM or OWL as it stands.
- **Lookup.** `core:classes` is resource-typed, so the triple index covers it. Equivalences are found
  through the `is_a` index entry for `core:EquivalentClasses`, once per chain head, and merged into
  groups (two equivalences sharing a class are one group). The subclass walk reads the groups from
  memory, never the index.
- **The layer for experiment 02.** One equivalence per UMLS concept that WordNet took and exactly one
  live HP code names: `{hp:…, wn:n…, umlscui:C…}`. `lexicon-align-hpo-emit` writes it beside the
  HPO↔UMLS alignment and `scripts/build-hpo-alignment-snapshot.sh` loads it above it. On HPO
  2026-09-01: 863 equivalences of the 942 concepts WordNet took; 53 skipped because the
  adjudication split the concept across synsets, 26 because no single live HP code names it.

**Found while tracing (fixed in code, takes effect at the reseed).** The UMLS importer mints three
entries per surface: `e_<CUI>_<i>`, `_mass`, and D70's named-condition `_name`
(`cat_n(C, name)`, for diseases and neoplasms). Both alignment emitters enumerated `["", "_mass"]`,
a list kept by hand, and read the number from a list that stopped at `pl`. Every named condition
they aligned kept its `_name` entry pointing at the UMLS concept. The parser saw both readings and,
for failure to thrive, kept the UMLS one. On the `uab-hpo-umls-aligned-2026-10-01` copy: 6,499
`_name` entries over 3,494 HP classes left out of the HPO↔UMLS layer, and 1,608 out of the
WordNet↔UMLS layer. `eigenius-umls` now exports `ENTRY_SUFFIXES`, both emitters enumerate through
`read::entries_of`, and the number is read from the category's `lexicon:Num` constructor.

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

1. **Disequality** (decision 10). Built first, because `variant:At` is a definition over `eigentt:field`: the `Apart` rule, the J rule, the codec and ESL forms, `proof_judgement` on any resource (D89 §2), the removal of `DecEq`, and `kernel/tests/disequality_by_field.rs`.
2. **Layers.** `ontologies/variant/variant.esl` and `ontologies/clinical/clinical.esl` are built and tested in `kernel/tests/d99_layers.rs` (11 tests):
   - resolutions as types;
   - `At` proved, and refused at the wrong position;
   - claim 6, and `instantiate` at `Prop`;
   - `Carries` of a nucleotide allele, the protein-only row as an existential, and `Carries` of a protein allele refused.

   `clinical` has no phenotype predicate yet. It waits on the parse of "the patient has polyuria".
3. **Reseed.** One reseed from a clean tree covers the OBO meta-ontology change of 4b07e11 and step 1's `core-ontology.json` and `justification.esl` changes. Record selections, ranks and `baseline.json` against the new snapshot. Required before a merge to main.
4. **Converter.** Convert `synthetic-cases.json` to an ESL layer without the parser. It applies decision 5's grounds, 6d's consent rule and §7's checks (HP label against the release history, HGVS consistency), and reports the HP:0004918 row.
5. **SYN-26-002 chain.** Encode the seven claims and the four items, with claim 6 restated per decision 3 and the 8a "no" resting on step 1.
6. **Integration test.** Add an ignored test against the snapshot. It asserts each claim's ground kind, the claim-6 refusal and the Verified disequality.
7. **README.** Write it in the experiment directory: the analyst table mapped to chain resources, and the supply table.
8. **Optional: parse prose.** Parse `Case History` and Notes prose with the HPO-aligned lexicon, under its own gate.

The PMI team's answers to the five questions of the shared write-up change converter rules only (step 4); the defaults are §7's and 6d's.

## Open

- **Phenotype-annotation form.** Read off a parse (2026-10-02, the `uab` kernel over `uab-hpo-umls-aligned-2026-10-01`). "He has hypernatremic dehydration." parses to `wn:v02203362_t(kind_of(hp:'0004906'), <he>)`.
  - The phenotype is a KIND: the class as a value, through `kind_of`, not an existential over instances.
  - The verb is WordNet's possession sense of *have*, `v02203362`.
  - Open: whether registry rows state this same form, or a `clinical:HasPhenotype` aligned to it, and whether *have* in its "suffer from" sense is the right verb.
- **The parser's class is not the registry's.** Five of SYN-26-002's six phenotypes parse to a class other than their HP code. Registry rows (HP codes) and parsed prose would not meet without a link between the two:

  | Phenotype | Registry | Parser |
  |---|---|---|
  | polyuria | HP:0000103 | `wn:n14114365` |
  | polydipsia | HP:0001959 | `wn:n14040966` |
  | nocturia | HP:0000017 | `wn:n13522485`, `umlscui:C0028734` |
  | renal insufficiency | HP:0000083 | `umlscui:C1565489` |
  | failure to thrive | HP:0001508 | `umlscui:C2315100`, `umlscui:C0015544` |
  | hypernatremic dehydration | HP:0004918 (wrong; HP:0004906) | `hp:'0004906'` |

  - **Failure to thrive and renal insufficiency** (C2315100, C1565489 among others) kept a `_name` entry pointing at the UMLS concept: traced and fixed (§11).
  - **Polyuria, nocturia, renal insufficiency** reach the WordNet synset: decision 11 links them.
  - **Polydipsia** had no link: the WordNet↔UMLS adjudicator judged UMLS polydipsia (C0085602,
    "chronic excessive intake of water") and WordNet's (n14040966, "excessive thirst") different
    (`same=false`, 0.75). The owner overrode it (2026-10-02): HPO HP:0001959 is "excessive thirst
    manifested by excessive fluid intake". The override is a recorded correction,
    `experiments/lexicon-align/maintainer-verdicts.jsonl`, that both resolve steps apply; the merge
    set grows 38,389 → 38,391.
- **Half of HPO did not stand bare** (found 2026-10-02 on `uab-d99-hpo-aligned-2026-10-02`). The
  UMLS importer mints the bare-standing `_name` entry (D70) only for diseases and neoplasms, so a
  symptom- or finding-typed concept had a count entry alone, and "he has polydipsia" could not take
  the HPO reading: 20,133 of 39,049 HPO-name surfaces, 8,918 of 18,419 HP classes. The owner's
  choice (2026-10-02): the HPO alignment adds a `_name` entry where a surface has neither `_mass`
  nor `_name`, because an HPO term is a named condition whatever its UMLS semantic type.
- **Result** (`uab-d99-r2-hpo-aligned-2026-10-02`):
  - the HPO↔UMLS layer is 81,223 entries; 864 equivalences;
  - all six SYN-26-002 phenotypes parse in "He has …" to their HP class or to a synset an
    equivalence links to it;
  - the parse gate holds: expected hits 62/62, readings 612 (652), skeletons 212, selection 30/41,
    0 unadjudicated. The selection score counts the owner's 2026-10-02 verdict that «analysed»
    v00644583 is a valid reading of «We analysed two independent cancer dependency data sets.»;
    rows 124, 142 and 163 were revised to match.
- **"The patient …" does not parse** ("The patient has polyuria.", "The patient has a fever."), while "The boy has polyuria." and "Patients have polyuria." do. A lexical gap in singular *patient*.
- **Threshold *k* in 8c.** The notes give <4% for p.Leu44Pro and "wild-type levels" for p.Leu44Phe.
- **The HP:0004918 row.** Report it to UAB, or declare HP:0004906 for the experiment.
- **Consent scope for coded variants** (6d).
