# PMI registry case — UAB Round 1, experiment 02

The package is `experiments/uab/UAB Round 1/02-synthetic-pmi-registry/`: three synthetic cases in
the shape of the PMI Case Registry's Airtable export, the registry schema, and an analyst's working
notes for SYN-26-002. It asks "is this the kind of case Eigenius can work, and what would you need
in hand to start?" (`README.md:11-12`). The check it sets is the notes' decomposition of SYN-26-002
into seven claims, each assigned Observed, Declared, Ill-typed or "Declared bridge"
(`SYN-26-002-analyst-notes.md:78-86`), and four items the analyst could not type (`:117-128`).

The design is `docs/design/d99-uab-registry-case-typing.md` (D99). This directory is its steps 4–7.

## Result

**The registry, converted without the parser** (`registry.esl`, from `pmi-registry-convert`). Three
cases, 44 resources, 35 declarations, each a `justification:Declaration` by the registry with the
case's medical records as its primary source. `registry-report.md` lists:

| | Item | Finding |
|---|---|---|
| held back | SYN-26-002 `HP:0004918` "Hypernatremic dehydration" | HP:0004918 is "Hyperchloremic metabolic acidosis"; the label is HP:0004906's, in all 18 releases checked |
| converted, stale | SYN-25-003 `HP:0003236` "Elevated circulating creatine kinase concentration" | the term's label 2021-06-08 … 2026-02-16; HPO 2026-09-01 keeps no synonym for it |
| converted | SYN-26-001 `HP:0100704`, `HP:0002353` | current synonyms; HP:0100704's was the label in 2018-03-08 |
| converted | SYN-26-001 SYNSYN3 `p.Gly88Ser` | protein change only: an existential over the nucleotide alleles that translate to it |
| converted, unsourced | SYN-26-001 SYNSYN3 RefSeq `NM_000003.2` | a RefSeq with no nucleotide change: declared without a primary source, builds no allele |
| not converted | SYN-25-003's two variants | no genomic-data-sharing consent |

**SYN-26-002's seven claims and four items** (`syngene.esl`, `syn-26-002.esl`,
`syn-26-002-conclusions.esl`). Each claim lands with its ground. Nothing is Observed, because no
tier-3 record is held. The kernel refuses three inferences (`refused.esl`):

| Refused | Reason the kernel reports |
|---|---|
| `Responsive(p.Leu44Pro)`, citing the survey | `no admitted IsDeclaredAs witness for IRI …:claim_6` — the survey says some responsive protein allele is at Leu44, and names none |
| `Responsive(c.131T>C)`, citing the survey | `… (is_a = [variant:NucleotideAllele]) does not inhabit class variant:ProteinAllele` — responsiveness is a protein-level property (D99 decision 3) |
| the chaperone agents reduce the child's urine output | `no admitted IsDeclaredAs witness for IRI …:chaperone_hypothesis` — a hypothesis has no trace, so nothing can cite it |

`crates/eigenius-pmi-registry/tests/syn_26_002_chain.rs` asserts the chain validates, the three
refusals with these reasons, and each claim's ground: it commits when cited by its ground and is
refused when cited as Observed. `syn_26_002_on_snapshot.rs` runs the same checks on the lexicon
snapshot, where the phenotype rows type-check against the real WordNet verb and HP classes.

## The analyst's table, on the chain

| # | Claim | Analyst's ground | Chain resource | Ground here | Would be Observed with |
|---|---|---|---|---|---|
| 1 | proband carries `c.131T>C`, hemizygous | Observed | `pmi:SYN_26_002_carries_NM_000010_7_131_C` | Declared, registry | the panel report (tier 3) |
| 2 | urine output did not fall on either trial | Observed; Declared for trial 1 | `syn002:trial_1_outcome`, `syn002:trial_2_outcome` | Declared, nephrologist | trial 2: the clinic notes' pre/post volumes; trial 1: none exist |
| 3 | `p.Leu44Pro` yields <4% of wild-type binding sites | Observed (someone else's) | `syngene:claim_3` | Declared, the assay's authors | the cited assay, resolved to a `reference:Reference` |
| 4 | responders achieved 12.6–31.6% reductions | Observed (someone else's) | `syngene:claim_4` | Declared, the survey's authors | the survey, resolved |
| 5 | "responsive" | Declared | `syngene:Responsive`, defined as a reduction of at least `syngene:survey_cutpoint` | Declared, the survey's authors (the cut-point's trace) | — a choice, not a measurement; the survey states its value |
| 6 | the survey's subject is our patient's variant | Ill-typed | `syngene:claim_6`: ∃q:ProteinAllele[At(q, Leu44)]. Responsive(q) | the claim is Declared; both substitutions are refused | — |
| 7 | chaperone agents might rescue trafficking | Declared bridge | `syngene:claim_7_cells` (cells) and `syn002:chaperone_hypothesis` (cells → child) | Declared, the cell-model authors; the step to the child is an `enc:Hypothesis` and grounds nothing | human data |

## The four untyped items

| Item | On the chain |
|---|---|
| **8a** the family's goal | Three `clinical:Question`s asked by the mother, traced to the registry's Participant/Family Goal unit. "Were the reports about her son's variant?" has the content ∀v. CohortCarried(survey cohort, v) → v = c.131T>C, and is answered by `syn002:not_his_variant`: Grounds(¬content), from the cohort carrying c.130C>T (Declared, the analyst's reading of the survey), c.130C>T ≠ c.131T>C (Verified, D99 §10) and the step joining them (Verified). "Why did it not work?" is answered by `syn002:no_response`. "Is there an alternative?" (∃x. ReducesOutput(x, proband)) is answered by the chaperone hypothesis. |
| **8b** absence from ClinVar | `clinvar:NotListed(release, c.131T>C)` and `clinvar:ListedAs(release, c.130C>T, pathogenic)` (`ontologies/clinvar/clinvar.esl`), about `syn002:clinvar_release`, a release nobody recorded. Declared by the registry from its Notes. Nothing derives a claim about the allele from them. |
| **8c** the judgment not to re-trial | `syn002:no_response`, a `justification:Conclusion`: the analyst's mechanism bridge, instantiated at the proband, c.131T>C and p.Leu44Pro, applied to claim 1 (carries, translates), `ProteinMediated(c.131T>C)` and claim 3. The trials are not among its grounds: they reach only the doses tried. `syn002:no_retrial` is a `clinical:Recommendation` resting on it. |
| **8d** trial 1's outcome | `syn002:trial_1_outcome`, `NoMeasurableReduction(trial_1)` Declared by the nephrologist; no volumes exist. |

## What UAB would supply

| To make Observed, or to complete | Needs | PHI |
|---|---|---|
| claim 1, and the lab's Likely Pathogenic classification | the panel report (tier 3) | yes |
| claim 2, trial 2 | clinic notes with pre/post volumes (tier 3) | yes |
| claim 2, trial 1 | volumes, which the notes say do not exist | — |
| 8b, ClinVar status | a real allele, and a release to query | no |
| claims 3, 4, 5, 7 and the cohort's allele | the cited papers, resolved to `reference:Reference` | no |
| a pathogenicity proposition | the condition each ACMG tier is for, and the classifying lab | no |
| 8c, the threshold below which the mechanism holds | the analyst's *k*; the bridge is declared at 4% | no |
| record identity in the live base | Airtable record ids | no |

## Choices made in building the chain

D99 does not settle these; each is open to revision.

- `ProteinMediated(c.131T>C)` is declared by the analyst. D99 8c lists it under the registry, but the registry has no field for it, and the notes' mechanism argument assumes it.
- `Responsive` is a definition over `syngene:survey_cutpoint`, an `axiom` whose value the package does not state, traced to the survey's authors. Claim 5's "somebody chose the cut-point" is that trace.
- The mechanism bridge is declared at 4%, the value the case needs. The notes state no threshold.
- The question "were the reports about her son's variant" is read as "is every allele the survey's cohort carried c.131T>C".
- A declaration whose own date is not in the package carries the date its source reached PMI (medical records received 2026-06-04, analyst work-up 2026-07-15), and says so in `prov:rationale`.
- The registry's ClinVar statements name one release, unrecorded.
- Genes are minted from the registry's symbol (`urn:eigenius:pmi:gene:<symbol>`), not resolved to `ncbi:Gene` (D99 §9). Alleles are `urn:eigenius:variant:allele:<accession>_<interval>_<alt>` (D99 §4), so `syngene.esl` mints c.130C>T and p.Leu44Phe under the converter's scheme.

## Running

```bash
# The registry layer and report (HPO: references/hpo/hp.json; history: scripts/provision-hpo-history.sh)
cargo run -p eigenius-pmi-registry --bin pmi-registry-convert -- \
  --cases "experiments/uab/UAB Round 1/02-synthetic-pmi-registry/synthetic-cases.json" \
  --out experiments/pmi-registry/registry.esl --report experiments/pmi-registry/registry-report.md

# The chain, WordNet and HPO stubbed
cargo test -p eigenius-pmi-registry

# The chain on the lexicon snapshot
EIGENIUS_DB_SNAPSHOT=../db-snapshot/uab-d99-r2-hpo-aligned-2026-10-02 \
  cargo test --release -p eigenius-pmi-registry --test syn_26_002_on_snapshot -- --ignored --nocapture
```

## Files

| File | Written by | Role |
|---|---|---|
| `registry.esl` | `pmi-registry-convert` | The three cases: individuals, cases, prose fields as `enc:DiscourseUnit`s, phenotype rows, alleles and their declarations |
| `registry-report.md` | `pmi-registry-convert` | What was held back, converted with a finding, or not converted |
| `syngene.esl` | hand | SYNGENE in the literature: Leu44, c.130C>T and p.Leu44Phe, the predicates, the survey, assay and cell-model sources, claims 3–7 |
| `syn-26-002.esl` | hand | The case beyond its row: the people, the lab's classification, inheritance, `ProteinMediated`, the trials and their outcomes, ClinVar, the cohort's allele, the disequality and the step joining it to the cohort (both proved), the mechanism bridge, the chaperone hypothesis |
| `syn-26-002-conclusions.esl` | hand | 8c's conclusion and recommendation; 8a's answer and the three questions |
| `refused.esl` | hand | The three refused inferences; never committed |

Chain order: the snapshot (or the bootstrap chain with stubs) → `ontologies/variant` →
`ontologies/clinical` → `ontologies/clinvar` → `registry.esl` → `syngene.esl` → `syn-26-002.esl`
→ `syn-26-002-conclusions.esl`. A certificate's cited witness must be in a committed parent, so
the conclusions sit one layer above what they cite.
