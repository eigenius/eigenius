// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! **PMI Case Registry → ESL** (UAB experiment 02, D99 step 4).
//!
//! Reads registry records in the shape of the PMI Case Registry's Airtable export
//! (`experiments/uab/UAB Round 1/02-synthetic-pmi-registry/synthetic-cases.json`) and writes an ESL
//! layer over the `variant` and `clinical` vocabularies. No parser: every field it converts has a
//! fixed shape, and free text is kept as discourse units for the parser to take up separately.
//!
//! What a value becomes follows D99:
//! - **§5 grounds.** Every proposition is a `justification:Declaration` by the registry, citing the
//!   case's tier-3 record as its source, with the `prov:DeclarationTrace` that admits it.
//! - **§2, §4 alleles.** A nucleotide allele, and the protein allele it translates to, get IRIs
//!   minted from their identity fields. A row with a protein change and no nucleotide change is an
//!   existential over the nucleotide alleles that translate to it.
//! - **§6d consent.** A case without study consent is not converted; without genomic-data-sharing
//!   consent its variant rows are not.
//! - **§7 checks.** An HPO label is checked against the term's labels across releases
//!   ([`hpo_history`]); `HGVS Notation` must equal `RefSeq:Genotype`. A row that fails is held back
//!   and reported, never converted on a guess.
//! - **§6, phenotypes.** `wn:v00065370_t(ontology:kind_of(<HP class>), <individual>)` — WordNet's
//!   *have*, "suffer from; be ill with", over the phenotype as a kind: the form the parser builds
//!   for "he has polyuria".

pub mod convert;
pub mod hgvs;
pub mod hpo_history;
pub mod record;
