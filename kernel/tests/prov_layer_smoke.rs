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

//! The `prov` bootstrap layer seeds and every declaration in it resolves.
//!
//! `prov` carries the provenance axis — Agent, Activity, the four provenance
//! Traces, and the relations between them — split out of `reflection` because
//! that ontology had come to hold two unrelated families under one word:
//! `reflection:Trace` with `LetTrace` / `MapTrace` / `CaseTrace` records how a
//! PROGRAM EVALUATED, while the parentless `DeclarationTrace` /
//! `ObservationTrace` / `ProductionTrace` / `VerificationTrace` record HOW A
//! RESOURCE CAME TO EXIST.
//!
//! The layer sits ABOVE `reflection` and that direction is forced:
//! `prov:ProgramTrace` points into the evaluation family through
//! `prov:trace_tree` and `reflection:output`, and nothing in `reflection` points
//! back.

use eigenius_kernel::ontology::iri::Iri;

#[test]
fn every_prov_declaration_resolves() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap seeds");
    for iri in [
        // agents
        "urn:eigenius:prov:Agent",
        "urn:eigenius:prov:Person",
        "urn:eigenius:prov:Organization",
        "urn:eigenius:prov:agent:unattributed",
        "urn:eigenius:prov:agent:eigenius_core_team",
        // activities
        "urn:eigenius:prov:Activity",
        "urn:eigenius:prov:used",
        "urn:eigenius:prov:was_associated_with",
        "urn:eigenius:prov:started_at",
        "urn:eigenius:prov:completed_at",
        // the core relations
        "urn:eigenius:prov:was_attributed_to",
        "urn:eigenius:prov:was_generated_by",
        "urn:eigenius:prov:had_primary_source",
        "urn:eigenius:prov:rationale",
        "urn:eigenius:prov:timestamp",
        // traces
        "urn:eigenius:prov:Trace",
        "urn:eigenius:prov:DeclarationTrace",
        "urn:eigenius:prov:ObservationTrace",
        "urn:eigenius:prov:ProductionTrace",
        "urn:eigenius:prov:ProgramTrace",
        "urn:eigenius:prov:VerificationTrace",
        "urn:eigenius:prov:resource",
        "urn:eigenius:prov:proof_system",
        "urn:eigenius:prov:proof_term",
        "urn:eigenius:prov:trace_tree",
    ] {
        let parsed = Iri::parse(iri).expect("well-formed IRI");
        assert!(
            ctx.resolve(&parsed).is_some(),
            "the prov layer must resolve `{iri}`"
        );
    }
}
