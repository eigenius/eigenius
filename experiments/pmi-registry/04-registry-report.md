# PMI registry conversion report

- Input: `experiments/uab/UAB Round 1/02-synthetic-pmi-registry/synthetic-cases.json`
- HPO: loaded release 2026-09-01; 17 earlier releases, 2018-03-08 … 2026-06-23
- Converted: 3 case(s), 44 resource(s), 35 declaration(s)

## Held back

| Case | Item | Finding |
|---|---|---|
| SYN-26-002 | HP:0004918 "Hypernatremic dehydration" | HP:0004918 is "Hyperchloremic metabolic acidosis" in 2026-09-01, and "Hypernatremic dehydration" is not its label in any of 17 earlier releases; "Hypernatremic dehydration" names HP:0004906 |

## Converted with a finding

| Case | Item | Finding |
|---|---|---|
| SYN-26-001 | HP:0100704 "Cortical visual impairment" | a synonym (hasExactSynonym) of the term in 2026-09-01; its label in 2018-03-08 |
| SYN-26-001 | HP:0002353 "Abnormal EEG" | a synonym (hasExactSynonym) of the term in 2026-09-01 |
| SYN-26-001 | Gene Info 3 (SYNSYN3 p.Gly88Ser) | protein change only: carried as an existential over the nucleotide alleles that translate to it (D99 §2) |
| SYN-26-001 | Gene Info 3 RefSeq NM_000003.2 | a RefSeq with no nucleotide change: declared without a primary source; builds no allele (D99 §7) |
| SYN-25-003 | HP:0003236 "Elevated circulating creatine kinase concentration" | stale: the term's label 2021-06-08 … 2026-02-16; 2026-09-01 labels it "Elevated circulating creatine kinase activity" and keeps no synonym for it |

## Not converted

| Case | Item | Finding |
|---|---|---|
| SYN-25-003 | 2 Gene Info row(s) | no genomic-data-sharing consent (D99 6d) |
| all | Participant Location | a region under a part-of relation (D99 §7); no region vocabulary |
| all | Ethnicity | an OMB category (D99 §6); no category vocabulary |
| all | Status, New Status Tags, Analyst Case Status, Case Category, Case Origin | workflow state with its timestamp (D99 §6); no workflow vocabulary |
| all | timestamps other than Case Created | the timestamp of a workflow state change (D99 §6); no workflow vocabulary |
| all | Outcome, Research Report Sent, Action Was Taken on Research Report | outcome records (D99 §6); no outcome vocabulary |
| all | Age, Age Calculation | not stored: age is computed from Year of Birth for a stated date (D99 §7) |
| all | Gene Info: Variant Type, Genomic Region | not typed by D99 |

