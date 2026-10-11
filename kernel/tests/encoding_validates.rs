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

//! The D62 `encoding` ontology (`ontologies/encoding/encoding.esl`) is **Expressible** and
//! **validates** over its documented parent chain (core → reflection/eigentt → logic →
//! lexicon-schema → reference). Until this test, the demo's `eig load` was the file's only
//! check — an edit could break it with nothing failing before the demo ran.
//!
//! Also pins the slice-5 selection vocabulary (d63-reading-selection.md): the
//! `enc:SelectionAuthority` enumeration is CLOSED — `enc:selected_by` carries `allows_only`
//! over exactly the three authority individuals (the `reflection:epistemic_status` pattern),
//! so a new authority is a deliberate vocabulary edit, never a silent mint.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;

fn json_layer(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn esl_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile(src, &parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile (not Expressible):\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("{name}: add_resource failed: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// core → reflection(+eigentt, institution, ingest) → prov → justification → logic →
/// lexicon-schema → reference — the chain `ontologies/encoding/encoding.esl`'s header documents it
/// loads after.
fn parent_chain() -> Arc<Layer> {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../ontologies/core/core-ontology.json")],
    );
    let refl = json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../ontologies/program/program-traces.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ],
    );
    // `prov` sits above reflection and below everything that names an agent, a
    // trace or an attribution — which is most of the stack.
    let prov = esl_layer("prov", include_str!("../../ontologies/prov/prov.esl"), refl);
    // `enc:EncodedClaim` subclasses `justification:Declaration` (D89 §3) — the parser's output is a
    // declaration whose accountable agent is the source document's authors or the pipeline's
    // operating agent, which is what `prov:was_attributed_to` on it already recorded.
    let justification = esl_layer(
        "justification",
        include_str!("../../ontologies/justification/justification.esl"),
        prov,
    );
    let logic = esl_layer(
        "logic",
        include_str!("../../ontologies/logic/logic.esl"),
        justification,
    );
    let lexicon = esl_layer(
        "lexicon-schema",
        include_str!("../../ontologies/lexicon/lexicon-ontology.esl"),
        logic,
    );
    esl_layer(
        "reference",
        include_str!("../../ontologies/reference/reference.esl"),
        lexicon,
    )
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

#[test]
fn encoding_ontology_is_expressible_and_validates() {
    let encoding = esl_layer(
        "encoding",
        include_str!("../../ontologies/encoding/encoding.esl"),
        parent_chain(),
    );
    let errors = Validator::new(Arc::clone(&encoding)).validate();
    assert!(
        errors.is_empty(),
        "encoding.esl validates with 0 errors, got {}:\n{errors:#?}",
        errors.len()
    );

    // The selection-authority enumeration is CLOSED: selected_by carries allows_only over
    // exactly the three authorities, and each authority individual is on the layer.
    let p = encoding
        .resolve(&iri("urn:eigenius:encoding:selected_by"))
        .expect("enc:selected_by is declared");
    let allows = p
        .get(&iri("urn:eigenius:core:allows_only"))
        .expect("selected_by carries allows_only (the closed-enumeration pattern)");
    let Value::Array(vals) = allows else {
        panic!("allows_only is an array, got {allows:?}");
    };
    let mut got: Vec<String> = vals
        .iter()
        // The IRI, not the variant that carries it — `Value::as_iri` parses. Nothing upgrades
        // a parsed `String` any more, because that upgrade never survived a storage round trip.
        .map(|v| {
            v.as_iri()
                .unwrap_or_else(|| panic!("allows_only entry should be an IRI, got {v:?}"))
                .as_str()
                .to_string()
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "urn:eigenius:encoding:authority_pin".to_string(),
            "urn:eigenius:encoding:authority_ranker".to_string(),
            "urn:eigenius:encoding:authority_sole".to_string(),
        ],
        "the authority enumeration is exactly pin | ranker | sole"
    );
    for a in ["authority_pin", "authority_ranker", "authority_sole"] {
        assert!(
            encoding
                .resolve(&iri(&format!("urn:eigenius:encoding:{a}")))
                .is_some(),
            "enc:{a} is on the layer"
        );
    }

    // The AnaphorBinding vocabulary (D67 §3): the binding-authority enumeration is CLOSED the
    // same way, and the machine-readable antecedent properties are declared.
    let p = encoding
        .resolve(&iri("urn:eigenius:encoding:bound_by"))
        .expect("enc:bound_by is declared");
    let allows = p
        .get(&iri("urn:eigenius:core:allows_only"))
        .expect("bound_by carries allows_only (the closed-enumeration pattern)");
    let Value::Array(vals) = allows else {
        panic!("allows_only is an array, got {allows:?}");
    };
    let mut got: Vec<String> = vals
        .iter()
        // The IRI, not the variant that carries it — `Value::as_iri` parses. Nothing upgrades
        // a parsed `String` any more, because that upgrade never survived a storage round trip.
        .map(|v| {
            v.as_iri()
                .unwrap_or_else(|| panic!("allows_only entry should be an IRI, got {v:?}"))
                .as_str()
                .to_string()
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "urn:eigenius:encoding:binding_proposer".to_string(),
            "urn:eigenius:encoding:binding_recency".to_string(),
            "urn:eigenius:encoding:binding_replay".to_string(),
        ],
        "the binding-authority enumeration is exactly recency | proposer | replay"
    );
    for a in [
        "AnaphorBinding",
        "binding_unit",
        "hole_var",
        "antecedent_surface",
        "antecedent_resource",
        "antecedent_term",
        "binding_recency",
        "binding_proposer",
        "binding_replay",
    ] {
        assert!(
            encoding
                .resolve(&iri(&format!("urn:eigenius:encoding:{a}")))
                .is_some(),
            "enc:{a} is on the layer"
        );
    }

    // The discourse-kind axis (D68 §2): the closed kind lattice under the enc:Claim root. Each
    // kind subclasses enc:Claim ON THIS LAYER; the per-kind lexicon alignment lives in the
    // separate chain-loaded claim-kind-alignment.esl (its targets are seeded sense classes this
    // chain does not have).
    for k in [
        "Claim",
        "Finding",
        "Observation",
        "Classification",
        "Hypothesis",
        "Suggestion",
        "Assertion",
    ] {
        assert!(
            encoding
                .resolve(&iri(&format!("urn:eigenius:encoding:{k}")))
                .is_some(),
            "enc:{k} is on the layer"
        );
    }
    for k in [
        "Finding",
        "Observation",
        "Classification",
        "Hypothesis",
        "Suggestion",
        "Assertion",
    ] {
        assert!(
            encoding.is_subclass_of(
                &iri(&format!("urn:eigenius:encoding:{k}")),
                &iri("urn:eigenius:encoding:Claim")
            ),
            "enc:{k} ⊑ enc:Claim"
        );
    }
}

/// D101's document structure holds an INSTANCE, not just a schema: `enc:Document` → `enc:body` →
/// `enc:Section` → `enc:children` → (`enc:Section` | `enc:Paragraph` | `enc:Table`) → `enc:units`
/// → `enc:DiscourseUnit`, with an `enc:ReferenceSpan` over one unit.
///
/// The load-bearing question is `enc:Block`. `enc:body` and `enc:children` are `class_types
/// enc:Block`, and every value they ever hold is a SUBCLASS instance — a Section, Paragraph or
/// Table, never a Block itself. If `class_types` did not walk the parent chain, the whole
/// containment design would be unusable, and the schema-only test above would not notice: it
/// validates declarations, and this is a fact about values. Heterogeneous children (a section
/// holding a sub-section and a table) are in the fixture for the same reason.
#[test]
fn the_document_structure_holds_a_nested_instance() {
    // `r##`, not `r#`: a Text Fragment locator opens with `#`, so `"#` inside the fixture
    // would close an `r#"` literal mid-ESL.
    const DOC: &str = r##"
namespace core      = "urn:eigenius:core";
namespace enc       = "urn:eigenius:encoding";
namespace reference = "urn:eigenius:reference";
namespace doc       = "urn:eigenius:test:doc";

resource doc:work : reference:Reference {
    reference:title = "A cited work";
}

resource doc:report : enc:Document {
    enc:document_title = "A two-section report";
    enc:media_type     = "text/markdown";
    enc:source_path    = "report.md";
    enc:source_sha256  = "deadbeef";
    enc:body           = [doc:sec_setting, doc:sec_finding_1];
    enc:back           = [doc:references];
}

// Back matter, as a real bibliography is shaped: a titled list holding prose AND a titled
// sub-group, not a flat array of works.
resource doc:references : enc:RefList {
    enc:title    = "References";
    enc:children = [doc:p_refs_provenance, doc:ref_group_1];
}

resource doc:p_refs_provenance : enc:Paragraph {
    enc:units = [doc:u4];
}

resource doc:u4 : enc:DiscourseUnit {
    enc:prose     = "Every PMID below was resolved against PubMed on 13 September 2026.";
    enc:unit_kind = enc:kind_prose;
}

resource doc:ref_group_1 : enc:RefList {
    enc:title = "XIAP deficiency - disease mechanism and phenotype";
    enc:refs  = [doc:work];
}

resource doc:sec_setting : enc:Section {
    enc:title         = "The variant and its setting";
    enc:section_kind  = enc:sec_intro;
    enc:children      = [doc:p1];
}

resource doc:p1 : enc:Paragraph {
    enc:units = [doc:u1];
}

resource doc:u1 : enc:DiscourseUnit {
    enc:prose     = "The residue is partially exposed on helix alpha-1.";
    enc:unit_kind = enc:kind_prose;
    enc:locator   = "#:~:text=The%20residue%20is%20partially%20exposed";
}

// A section whose children are a SUB-SECTION and a TABLE — heterogeneous, both Blocks.
resource doc:sec_finding_1 : enc:Section {
    enc:title         = "Finding 1 - the variant is absent from every resource queried";
    enc:section_kind  = enc:sec_results;
    enc:children      = [doc:sec_sub, doc:t1];
}

resource doc:sec_sub : enc:Section {
    enc:title         = "Callability, not tolerability";
    enc:section_kind  = enc:sec_results;
    enc:children      = [doc:p2];
}

resource doc:p2 : enc:Paragraph {
    enc:units = [doc:u2];
}

resource doc:u2 : enc:DiscourseUnit {
    enc:prose     = "Thirteen of seventeen predictors rate it worse (Rigaud et al. 2006).";
    enc:unit_kind = enc:kind_prose;
    enc:locator   = "#:~:text=Thirteen%20of%20seventeen";
}

resource doc:t1 : enc:Table {
    enc:units = [doc:u3];
}

resource doc:u3 : enc:DiscourseUnit {
    enc:prose     = "| predictor | call | score |";
    enc:unit_kind = enc:kind_table;
}

resource doc:xref_1 : enc:ReferenceSpan {
    enc:span_unit   = doc:u2;
    enc:ref_target  = doc:work;
    enc:ref_type    = enc:ref_bibr;
    enc:ref_surface = "Rigaud et al. 2006";
    enc:locator     = "#:~:text=worse%20-,(Rigaud%20et%20al.%202006)";
}
"##;

    let encoding = esl_layer(
        "encoding",
        include_str!("../../ontologies/encoding/encoding.esl"),
        parent_chain(),
    );
    let instance = esl_layer("test-doc", DOC, encoding);
    let errors = Validator::new(Arc::clone(&instance)).validate();
    assert!(
        errors.is_empty(),
        "a nested document instance validates with 0 errors, got {}:\n{errors:#?}",
        errors.len()
    );

    // The tree is walkable by containment alone: the document's body reaches the units without
    // any unit carrying a back-pointer. This is what replaces `enc:section` as a string.
    let report = instance
        .resolve(&iri("urn:eigenius:test:doc:report"))
        .expect("doc:report is on the layer");
    let Some(Value::Array(body)) = report.get(&iri("urn:eigenius:encoding:body")) else {
        panic!("the document has an ordered enc:body");
    };
    assert_eq!(body.len(), 2, "two top-level sections, in document order");
    assert_eq!(
        body[0].as_iri().expect("body entries are IRIs").as_str(),
        "urn:eigenius:test:doc:sec_setting",
        "document ORDER is array order — the setting section is first"
    );

    // A section's kind is a resource on the layer, so "the methods" is a query over
    // enc:section_kind rather than a match on a string.
    let sec = instance
        .resolve(&iri("urn:eigenius:test:doc:sec_finding_1"))
        .expect("the finding section resolves");
    assert_eq!(
        sec.get(&iri("urn:eigenius:encoding:section_kind"))
            .and_then(|v| v.as_iri())
            .expect("the section carries a kind")
            .as_str(),
        "urn:eigenius:encoding:sec_results",
        "a numbered finding is typed `results`; its number lives in the title and its identity"
    );
    // Back matter nests: the bibliography is a titled RefList whose children include prose and a
    // titled sub-group. A flat enc:ref_list property could hold none of that, which is why the
    // first cut of this vocabulary was wrong — the XIAP report's bibliography has a provenance
    // paragraph, four titled groups over 63 entries, and a trailing accessions paragraph.
    let refs = instance
        .resolve(&iri("urn:eigenius:test:doc:references"))
        .expect("the bibliography resolves");
    assert_eq!(
        refs.get(&iri("urn:eigenius:encoding:title"))
            .and_then(|v| v.as_str().map(str::to_string))
            .expect("the RefList is titled"),
        "References"
    );
    let Some(Value::Array(kids)) = refs.get(&iri("urn:eigenius:encoding:children")) else {
        panic!("the RefList nests prose and sub-groups");
    };
    assert_eq!(kids.len(), 2, "a Paragraph and a sub-RefList, both Blocks");

    for k in ["sec_intro", "sec_methods", "sec_results", "sec_conclusions"] {
        assert!(
            instance
                .resolve(&iri(&format!("urn:eigenius:encoding:{k}")))
                .is_some(),
            "enc:{k} is on the chain"
        );
    }
}
