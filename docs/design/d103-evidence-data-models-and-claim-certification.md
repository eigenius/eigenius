# D103 — Evidence Data Models, Human-Verified Readings, and End-to-End Claim Certification

**Status: decided (design consolidated, ready for phased implementation).**
Extends the `D66 → D70 → D71 → D93 → D99 → D101 → D102` pipeline from **formulated prose claims** (`IsDeclaredAs(claim, P)`) to **end-to-end evidence-backed certification** (`Grounds(P)`) across prose, tables, citations, raw data archives, external script outputs, and multi-pass document revisions.

---

## 1. Problem & Target State

Today, the encoding pipeline (`prose-to-esl`, D71) ends at the **formulation boundary**:

- It parses body-prose Controlled Natural Language (CNL) sentences into closed `EigenTT` propositions over WordNet + UMLS + HPO + Stage-A document-local individuals (`doc:ni_*`).
- Every landed `enc:EncodedClaim` commits under a `prov:DeclarationTrace` (`IsDeclaredAs(claim_i, P)`), with `gate_sentence = None` and no `justification:Grounds(P)` certificate.
- Non-prose document elements (`kind_table` rows, `enc:Ref` bibliography entries, section headings), raw external database snapshots (`JSON`/`VCF`/`PDB`/`XML`), acquisition provenance (HTTP exchanges including `4xx` negative results), external analysis scripts, cross-claim dependency graphs, and multi-pass amendment histories sit outside the encoded claims layer.

**Target state:** A layered, kernel-validated chain architecture covering the full lifecycle of a certified scientific or clinical document:

```text
Layer 0   Bootstrap + lexicon snapshot (WordNet + UMLS + HPO, aligned)
Layer 1   Chain-loaded domain data-model ontologies (zero bootstrap edits):
            variant → clinical → clinvar, popgen, structure, predictors, sequence, acquisition
Layer 2   Document model (`enc:Document` tree per D101/D102: sections, paragraphs, tables, refs)
Layer 3   Human-verified prose claims (`enc:EncodedClaim` via sense-resolved pins, 0 cuts, attributed)
            + direct structured table-row resources (`kind_table` → atomic propositions, bypassing DCG)
            + bibliography / source resources (`reference:Reference`, negative-retrieval records)
Layer 4   Ingested raw data & acquisition cache (`prov:Activity` + measurement resources + `prov:ObservationTrace`)
Layer 5   Entity alignment & claim-to-data certification bridges:
            doc:ni_* ↔ typed domain resources (explicit attributed bridges)
            def abbreviations / Declaration bridges / script derivations → `justification:Conclusion` (`Grounds(P)`)
Layer 6+  Multi-pass revision layers (Pass 1 → Pass 2 → Pass 3; retired/superseded claims & gate checks)
```

---

## 2. Settled Architectural Decisions

| # | Decision | Choice | Rationale |
|---|---|---|---|
| **D1** | **Reading-level human verification pins** | Add a **sense-resolved reading pin** mechanism keyed on `(unit / sentence, chosen_sem)` (or curated selection overlay) alongside `reading-adjudications.tsv`. | Existing `--pins` (`PinReadingRanker`) matches sense-erased skeletons and fails closed on sense ties; human sign-off requires pinning exact sense-bearing `EigenTT` terms while remaining 100% deterministic offline. |
| **D2** | **Entity alignment seam (`doc:ni_*` ↔ domain IRIs)** | **Separate post-parse alignment layer via explicit attributed bridges** (rather than resolving external DB IRIs inside Stage-A glossary seeding). | Preserves D70's syntax-vs.-denotation separation: keeps `prose-to-esl` and recorded LLM draws (`ranks.json`/`selections.json`) decoupled from database re-ingestion, avoids hiding non-injective or isoform-dependent identity claims (`ProteinAllele ↔ NucleotideAllele`) inside tokenization, and makes every resolution auditable with a `prov:Agent`. |
| **D3** | **Table claims (`kind_table`)** | **Direct structured resources carrying atomic propositions** — each table row becomes a typed resource on the chain, bypassing the DCG prose parser. | Table cells carry structured key/value observations (counts, scores, coordinates, classifications) where prose rewriting adds ambiguity without adding meaning; direct emission guarantees 1-to-1 structural fidelity and clean joins to raw data records. |
| **D4** | **External script outputs & arithmetic (v1)** | In v1, derived computations produced by external analysis scripts may land as **`Declared` rules** (`App(Declared(script_rule), Observed(raw))`) or **`Observed` script-output artifacts** (`prov:ObservationTrace` citing the script activity and archived output). | Avoids blocking end-to-end certification on porting every analysis script into an in-kernel or external verification institution; keeps the script, inputs, command, and output hash explicitly inspectable on the chain, with a clean upgrade path to dispatched institutions (`IsVerifiedAs`) in v2. |
| **D5** | **Scope** | Target the **full scope** across all six data-model ontology modules, table/reference coverage, human-verified prose, cross-claim dependencies, and multi-pass revision history, delivered in testable vertical slices. | Designing all six modules together up front ensures shared identity types (`variant:Allele`, `variant:Location`, `units:Quantity`, `acquisition:HttpExchange`) compose cleanly across domains before per-source ingestors are written. |

---

## 3. Human-Verified Reading Baseline (Prose Layer)

Every prose proposition must continue to originate from the lexicon-grounded DCG parser — never hand-written ad-hoc predicates — so meaning remains in the ontology types rather than in opaque predicate names.

### 3.1 Sense-Resolved Pin Format & Pipeline Seam

Extend `eigenius-encoding` (`select.rs`, `pipeline.rs`, and `kernel/src/dcg/reading_ranker.rs`) with a `--reading-pins <tsv>` (or curated-overlay) arm that reads rows with verdict `correct` from `reading-adjudications.tsv`:

```text
unit_id <TAB> sentence <TAB> chosen_sem <TAB> verdict <TAB> reviewer_basis
```

- **Exact match semantics:** Within the candidate pool for `unit_id` (matching `sentence`), select the unique candidate whose normalized `pretty_term(item.sem())` equals `chosen_sem`.
- **Fail-closed guarantees:** `0` matches or `>1` matches aborts under strict mode (surfacing grammar/lexicon drift immediately); any unit without a `correct` pin falls through to the replayable ranker draw (`--selections`) during incremental curation, and **fails closed** once strict `--require-all-pinned` is enabled.
- **Companion curation for anaphora & kinds:** Curated overrides to `proposals.json` (antecedent binding) and `kinds.json` (D68 rhetorical kind) remain in the existing replay draw files so all four seams stay byte-reproducible with `0` live LLM calls.

### 3.2 Front-to-Back Adjudication Workflow & Exit Gates

1. Review each source unit against the **original prose** (not only the CNL), inspecting both `Register::Surface` and D69 `Register::Expanded` verbalizations (exposing WordNet/UMLS/HPO/`doc:*` sense IDs), resolved `enc:AnaphorBinding`s, and D68 `Claim` kinds.
2. Triage every `wrong` or `CutItem` sentence into its authoritative upstream fix:
   - *Pool error / abstention / refusal:* pin the faithful candidate in `--reading-pins` (or override `proposals.json`/`kinds.json`).
   - *Missing domain sense:* add an explicit definition in the Stage-A glossary source or adjust `ranks.json`.
   - *CNL drift or avoidable homograph:* tighten the CNL rewrite source and rebuild.
   - *Forest-wide composition defect:* fix the general combinator/seed rule in `kernel/src/dcg/` (gated against the core-en construction battery).
3. Proceed **front-to-back by document section**, re-running deterministic offline replay after each section so downstream discourse anaphors bind against verified upstream claims.
4. **Exit gate:** `prose-to-esl` without `--partial` emits `0` `enc:CutItem`s, `reading-adjudications.tsv` has `0` `wrong` and `0` `uncertain` rows, `--verify-load` passes with `0` `Validator` errors, and `--declared-by <prov:Agent IRI>` replaces `agent:unattributed` on every `prov:DeclarationTrace`.

---

## 4. Document-Tree Anchoring & Non-Prose Ingestion (Completing D102)

### 4.1 Anchoring Prose Claims to `enc:Document`

Complete the D102 ingest seam so `prose-to-esl` accepts a chain-resident `enc:Document` IRI (or `--doc-map` mapping CNL unit indices back to D101 `enc:DiscourseUnit` IRIs) rather than minting an unanchored synthetic `:source`:

- `enc:ReasoningStructure.enc:document` points to the `enc:Document`.
- Each emitted `enc:EncodedClaim` links via `enc:from_unit` to the D101 `enc:DiscourseUnit` (preserving `enc:section`, `enc:locator`, `enc:span_start`/`end`, and `enc:unit_kind`).

### 4.2 Direct Table-Row Resources (Decision D3)

For every `kind_table` unit in the `enc:Document`:

- Emit a typed table-row resource (subclass of `enc:TableRow` / domain record class) carrying:
  - structural anchor (`enc:from_unit` pointing to the D101 table cell/row unit),
  - typed column properties (e.g., `variant:Allele`, `units:Quantity`, `clinical:AcmgTier`, predictor metrics), and
  - a closed, lexicon/domain-typed `eigentt:proposition` derived directly from the structured row (e.g., `clinvar:ListedAs(...)`, `predictors:HasScore(...)`, `popgen:ObservedCounts(...)`) plus its `prov:DeclarationTrace` attributed to the document's author/agent.
- Table-row resources never pass through the DCG parser, eliminating the 100+ spurious table grammar gaps identified in D102.

### 4.3 Bibliography & Source Units (`§References`)

- `enc:Ref` entries in the D101 back matter map directly to `reference:Reference` resources carrying identifiers (`pmid`, `pmcid`, `doi`, `url`, `sha256` of archived full text or abstract).
- Unretrievable or paywalled sources (negative retrieval logs) land as `reference:Reference` resources annotated with a `prov:Activity` (`acquisition:HttpExchange`) recording the attempted URLs and HTTP status (`403`/`404`/paywall), preserving scope transparency without minting false observations.

---

## 5. Domain Data-Model Ontologies (Chain-Loaded ESL)

All six modules load on top of the immutable bootstrap chain plus `ontologies/variant/variant.esl` and `ontologies/clinical/clinical.esl` (zero bootstrap edits, zero reseeds). Every module follows the **D99 release-and-observation pattern** established in `ontologies/clinvar/clinvar.esl`:

1. **Snapshot / Release class:** Fixes *which* database state was queried.
2. **Typed record class + properties:** Validates field types, required keys, closed enums (`allows_only`), and dimensional units (`units:Quantity`).
3. **Atomic observation predicates (`data ... : Prop`):** States *only* what the raw artifact contains — never conflating a database lookup with a clinical conclusion.

### 5.1 `ontologies/acquisition/acquisition.esl` (Phase-1 Fetch Provenance)

Models the Phase-1 acquisition log (`INDEX.jsonl` + archived response bodies) over `prov:Activity`:

- **Classes:** `acquisition:HttpExchange : prov:Activity`, `acquisition:ArchivedArtifact : core:Resource`.
- **Properties:** `acquisition:url`, `acquisition:http_method`, `acquisition:request_body_sha256`, `acquisition:http_status : core:integer` (optional on transport timeout/DNS error), `acquisition:outcome` (`allows_only` `{ok, http_error, transport_error}`), `acquisition:status_source` (`allows_only` `{observed, inferred, unknown}`), `acquisition:body_sha256`, `acquisition:byte_length : core:integer`.
- **Atomic predicates:**
  - `data acquisition:ReturnedStatus : acquisition:HttpExchange -> core:integer -> Prop`
  - `data acquisition:FetchFailed : acquisition:HttpExchange -> Prop`
- **Why first-class HTTP status matters:** Negative and endpoint-behavior claims (e.g., an API returning `404` for an unlisted record or `401` unauthenticated) ground directly on `ReturnedStatus` via `prov:ObservationTrace`, keeping negative evidence deterministic offline.

### 5.2 `ontologies/clinvar/clinvar.esl` (Extension of Existing Module)

Retains existing `clinvar:Release`, `clinvar:NotListed`, and `clinvar:ListedAs`, and adds structured variation records:

- **Classes:** `clinvar:VariationRecord : lexicon:Entity`, `clinvar:ReviewStatus` (closed enum mapping ClinVar's 0–4 star tiers).
- **Properties:** `clinvar:vcv_accession : core:string`, `clinvar:vcv_version : core:integer`, `clinvar:record_release` (`clinvar:Release`), `clinvar:record_allele` (`variant:Allele`), `clinvar:classification` (`clinical:AcmgTier`), `clinvar:review_status` (`clinvar:ReviewStatus`), `clinvar:star_rating : core:integer`, `clinvar:submitter_count : core:integer`, `clinvar:has_conflict : core:boolean`.
- **Atomic predicates:**
  - `data clinvar:HasReviewStars : clinvar:Release -> variant:Allele -> core:integer -> Prop`
  - `data clinvar:HasSubmitterCount : clinvar:Release -> variant:Allele -> core:integer -> Prop`
  - `data clinvar:HasConflict : clinvar:Release -> variant:Allele -> Prop`

### 5.3 `ontologies/popgen/popgen.esl` (Population Genetics & Constraint)

Covers population-frequency databases (gnomAD GraphQL sweeps + release VCFs, All of Us, ToMMo, BRAVO) and gene/regional missense constraint:

- **Classes:**
  - `popgen:DatasetRelease : lexicon:Entity` (`dataset_name`, `version`, `reference_genome`, `callset` ∈ `{exome, genome, joint, cohort}`)
  - `popgen:AlleleObservation : lexicon:Entity` (`release`, `allele : variant:NucleotideAllele`, `ac`, `an`, `ac_hemi`, `ac_hom`, `ac_xy`, `an_xy`, `nhomalt_xy`, `af : units:Quantity`, `filter_pass : core:boolean`)
  - `popgen:GeneConstraint : lexicon:Entity` (`release`, `gene_symbol`, `transcript_accession`, `pli`, `oe_lof`, `oe_mis`, `obs_mis`, `exp_mis`)
  - `popgen:RegionalConstraintWindow : lexicon:Entity` (`release`, `location : variant:Location`, `obs_mis`, `exp_mis`, `oe_ratio`, `p_depletion`, `passed_qc : core:boolean`, `has_no_rmc_evidence : core:boolean`)
- **Atomic predicates:**
  - `data popgen:ObservedAlleleCount : popgen:DatasetRelease -> variant:NucleotideAllele -> units:Quantity(u"1") -> Prop`
  - `data popgen:ObservedHemizygousCount : popgen:DatasetRelease -> variant:NucleotideAllele -> units:Quantity(u"1") -> Prop`
  - `data popgen:AbsentFromDataset : popgen:DatasetRelease -> variant:NucleotideAllele -> Prop`
  - `data popgen:RegionalDepletion : popgen:DatasetRelease -> variant:Location -> units:Quantity(u"1") -> Prop`
- **Design safeguard:** `AlleleObservation` and `Observed*` predicates require `variant:NucleotideAllele` (never `ProteinAllele`), preserving D99/D100's resolution refusal at type-check time. `passed_qc` and `has_no_rmc_evidence` are modeled as separate boolean fields so *"searched with no constrained regions found"* cannot be collapsed into *"QC failure"*.

### 5.4 `ontologies/structure/structure.esl` (Protein Annotations & 3D Structure)

Covers UniProtKB entries, experimental PDB/PDBe structures, and AlphaFold models:

- **Classes:**
  - `structure:ProteinEntry : lexicon:Entity` (`uniprot_accession`, `gene_symbol`, `sequence_length`)
  - `structure:ProteinDomain : lexicon:Entity` (`protein`, `domain_name`, `location : variant:Location`)
  - `structure:StructureModel : lexicon:Entity` (`model_id`, `source_db` ∈ `{pdb, alphafold, model}`, `experimental_method`, `resolution : units:Quantity`)
  - `structure:ResidueMetric : lexicon:Entity` (`model`, `location : variant:Location`, `sasa : units:Quantity`, `relative_sasa : units:Quantity`, `plddt : units:Quantity`)
  - `structure:AtomContact : lexicon:Entity` (`model`, `residue_a : variant:Location`, `residue_b : variant:Location`, `atom_a`, `atom_b`, `distance : units:Quantity`, `includes_backbone : core:boolean`)
- **Atomic predicates:**
  - `data structure:ContactsAtDistance : structure:StructureModel -> variant:Location -> variant:Location -> units:Quantity(u"m") -> Prop`
  - `data structure:ResidueExposed : structure:StructureModel -> variant:Location -> units:Quantity(u"1") -> Prop`
- **Dimensional consistency:** Distances use `units:Quantity(u"m")` (normalized from Ångströms via D93), matching the exact unit emitted by the DCG parser for prose distance statements.

### 5.5 `ontologies/predictors/predictors.esl` (In-Silico Predictors & MAVE Assays)

Covers MyVariant/dbNSFP aggregated scores, CADD, SpliceAI, REVEL, AlphaMissense, and MaveDB:

- **Classes:**
  - `predictors:ToolRelease : lexicon:Entity` (`tool_name`, `version`, `aggregator`)
  - `predictors:VariantScore : lexicon:Entity` (`tool`, `allele : variant:Allele`, `metric_name`, `raw_score : units:Quantity`, `rank_score : units:Quantity`, `categorical_call`)
  - `predictors:MaveScoreSet : lexicon:Entity` (`mavedb_urn`, `target_gene`, `assay_type`)
- **Atomic predicates:**
  - `data predictors:ScoredAs : predictors:ToolRelease -> variant:Allele -> core:string -> units:Quantity(u"1") -> Prop`
  - `data predictors:NoMaveScoreSet : predictors:ToolRelease -> core:string -> Prop`

### 5.6 `ontologies/sequence/sequence.esl` (Sequence, Conservation, Editing Windows & dbSNP)

Builds on `variant:SequenceReference`, `variant:Transcript`, `variant:Location`, and `variant:NucleotideAllele`:

- **Classes:**
  - `sequence:TranscriptAnnotation : lexicon:Entity` (`transcript : variant:Transcript`, `mane_select : core:boolean`, `cds_length`, `exon_count`)
  - `sequence:OrthologConservation : lexicon:Entity` (`location : variant:Location`, `clade`, `species_count`, `conserved_count`, `alignment_source`)
  - `sequence:EditingWindow : lexicon:Entity` (`transcript : variant:Transcript`, `editor_name`, `protospacer_seq`, `pam_seq`, `target_allele : variant:NucleotideAllele`, `bystander_alleles : core:resource_array`)
  - `sequence:DbSnpRecord : lexicon:Entity` (`rsid`, `allele : variant:NucleotideAllele`, `reference_genome`)
- **Atomic predicates:**
  - `data sequence:ConservedInClade : variant:Location -> core:string -> units:Quantity(u"1") -> Prop`
  - `data sequence:CoEditableBystander : sequence:EditingWindow -> variant:NucleotideAllele -> Prop`

---

## 6. Entity Alignment & Claim-to-Data Certification Bridges

### 6.1 Separate Entity Alignment Layer (Decision D2)

Between the parsed prose layer (`doc:ni_* : lexicon:Entity`) and the ingested data layer (`variant:*`, `structure:*`, `clinvar:*`), commit a dedicated **alignment layer** containing explicit, agent-attributed alignment declarations:

- **1-to-1 denotation alignment (same resolution):** When a document token names an exact domain entity at matching resolution (e.g., `doc:ni_p_x123y` ↔ `variant:ProteinAllele`, `doc:ni_x123` ↔ `variant:Location`, `doc:ni_vcv...` ↔ `clinvar:VariationRecord`), emit a `justification:Declaration` asserting the link (`align:DenotesEntity(doc:ni_..., domain:res)` or `eigentt:Eq` where sorts match), proposed mechanically by a deterministic notation/accession matcher and attributed to an accountable `prov:Agent`.
- **Cross-resolution designation bridges (non-injective):** When prose names a `ProteinAllele` or `Location` while the underlying data counts a `NucleotideAllele`, emit two distinct declarations (per D100):
  1. Biological translation: `variant:TranslatesTo(nuc_allele, prot_allele)` (computed if transcript sequence is loaded, else `Declared`),
  2. Editorial designation: `align:DesignationDenotes(prot_allele, nuc_allele)` — explicitly marking the non-injective reading choice that the kernel refuses to infer silently.

### 6.2 Connecting Atomic Observations to Parsed Propositions

Every claim in the certification register connects to ingested evidence via one of three typed mechanisms:

1. **Transparent `def` abbreviations (D66 pattern, purely definitional):**
   - Where a structured data predicate is definitionally equivalent to a parsed WordNet/UMLS/units proposition (e.g., residue contact distance in meters, ClinVar tier listing), write a transparent `def` over the parser's lexicon so the kernel equates the data-side and prose-side propositions by conversion — zero unchecked declarations needed.
2. **External script outputs & arithmetic (Decision D4, v1):**
   - **Rule-application shape (`App(Declared(rule), Observed(raw))`):** When a script aggregates raw exports (e.g., summing exome + genome counts, reconciling VCF totals, sliding-window expected counts), commit the raw inputs under `prov:ObservationTrace` (`IsObservedAs`) and the script's rule under `justification:Declaration` citing the script path, command, and output hash as `prov:Source`.
   - **Observed-output shape (`Observed(script_output)`):** Where an analysis script produces a primary derived artifact (e.g., regional-constraint table) treated as an archived measurement output in v1, commit the script run as a `prov:Activity` (`prov:used` = raw inputs) and the output artifact under `prov:ObservationTrace`.
   - Both forms compose identically under `Grounds.app` and upgrade cleanly to dispatched verification institutions (`IsVerifiedAs`) in v2 without altering downstream conclusions.
3. **Multi-premise & cross-claim derivations (`marked` claims and implicit `depends_on`):**
   - Extract cross-claim premise references (both structured `depends_on:`/`premises:` fields and prose-cited claim IDs) into explicit `justification:Conclusion` resources applying declared domain bridge implications (`A₁ → … → Aₖ → B`) via nested `Grounds.app`.
   - Because `Grounds(P)` is indexed by the normalized proposition `P`, a sound inference over a rejected or blocked premise fails type-checking **naming the exact absent leaf witness**, while independently grounded sibling limbs still commit.

---

## 7. Multi-Pass Revision Layers & Unmodeled Warrant Kinds

### 7.1 Multi-Pass Chain Evolution (`Pass 1 → Pass 2 → Pass 3`)

Represent each certification pass as an immutable Eigenius layer extending the shared data-model ontologies:

- **Pass 1 layer:** Anchors to the Pass-1 `enc:Document` snapshot and includes both admitted claims and isolated validation test layers for Pass 1's rejected and blocked claims (mirroring D100's negative tests: ill-typed resolution mismatches, missing `IsObservedAs` witnesses, missing premise leaves).
- **Pass 2 / Pass 3 layers:** Anchor to the amended `enc:Document` snapshots, recording `supersedes` / `retired` provenance links on replaced claims and verifying that amended propositions and new evidence layers commit with `0` `Validator` errors.
- **Replication check:** Running `Validator::validate()` across the chain layers acts as the kernel-checked counterpart to `make check` + `make replicate`, with cryptographic/layer immutability replacing the external byte-prefix script.

### 7.2 Boundary Treatment for `compressed` and `exempt`

- **`compressed` (section headings and summary claims):** D100 established that logical entailment (`A ∧ Q → A`) *admits* a heading that drops a critical caveat, whereas certification rejects it. In v1, model `compressed` claims explicitly with a `justification:CompressionRecord` (carrying `compresses : core:resource_array` and `required_qualifiers : core:resource_array` citing the qualifier propositions/records that must remain present or explicitly discharged in the summary) and enforce a **non-droppable-qualifier validation check** as a dedicated structural rule outside `Grounds` entailment, clearly separating *logical soundness* from *summary faithfulness*.
- **`exempt` (scope caveats and framing):** Add an explicit `enc:ExemptUnit` / `justification:ExemptionRecord` carrying `reason` and attribution (`prov:was_attributed_to`), marking units deliberately outside the warrant gate rather than silently omitting them.

---

## 8. Phased Implementation Plan & Verification Gates

Every phase produces a self-contained, kernel-validated deliverable (`cargo test`, `Validator::validate() == 0` errors, zero private corpus leakage into tracked files):

| Phase | Deliverable | Verification Gate |
|---|---|---|
| **Phase 1 — Domain data-model ontologies (all 6 modules + `align`)** | `acquisition.esl`, extended `clinvar.esl`, `popgen.esl`, `structure.esl`, `predictors.esl`, `sequence.esl`, and `align.esl` (`DenotesEntity`, `DesignationDenotes`, `CompressionRecord`, `ExemptionRecord`). | All modules compile and pass `Validator::validate()` over `bootstrap + variant + clinical` with positive & negative type-refusal tests (e.g., `ProteinAllele` rejected by `popgen:ObservedHemizygousCount`). |
| **Phase 2 — Data, cache, manuscript & table ingestors** | Deterministic ingestors converting archived `cache/INDEX.jsonl`, all 6 `data/` families, `manuscripts/` (+ `MISSING.md`), and `kind_table` rows into gitignored ESL layers + `prov:ObservationTrace`s. | `eigenius load` / `Validator` `0` errors across all ingested resources; `100%` of cached fetches (including non-200) and table rows landed with SHA-256 provenance. |
| **Phase 3 — Sense-resolved pins, D102 anchor & human-verified prose** | Add `--reading-pins`, `--declared-by`, and `enc:Document` unit-ID anchoring (`D102`) in `eigenius-encoding`; adjudicate prose section-by-section into `reading-adjudications.tsv` + glossary/CNL/grammar fixes. | Fail-closed emission (`0` `CutItem`s), `0` `wrong`/`uncertain` rows, `0` `Validator` errors, human `prov:Agent` attribution, offline replay byte-identical. |
| **Phase 4 — Entity alignment & full claim-to-data bridges** | Alignment declarations (`doc:ni_* ↔ domain:*`), transparent `def` bridges, `App(rule, Observed(data))` / `Observed(script_output)` conclusions, and extracted cross-claim dependency graph (`depends_on` + multi-premise `Conclusion`s). | Every live claim's `Grounds(P)` / `CompressionRecord` / `ExemptionRecord` validates on the chain with `0` errors. |
| **Phase 5 — Multi-pass revision & negative-gate suite** | `Pass 1 → Pass 2 → Pass 3` chain layers plus automated negative tests confirming Pass 1's rejections/blocks fail admission for their exact structural causes and Pass 2/3's amendments pass. | End-to-end regression suite passing `100%` offline with zero network access. |
