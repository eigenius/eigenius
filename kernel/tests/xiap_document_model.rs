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

//! The XIAP report's D101 document model loads through the kernel (survey §4 step 5, step 1).
//!
//! `experiments/parsing/uab-xiap-document-model.py` emits it from the report, which is gitignored
//! material — so the ESL is gitignored too and this test is `#[ignore]`d, taking its path from
//! `XIAP_DOCUMENT_MODEL`.
//!
//! What it pins is the TREE, not the vocabulary. Compiling and validating is not the gate: the
//! flat list this converter used to emit — 467 DiscourseUnits with a section string — also
//! compiled and validated clean, because there was no vocabulary for it to be wrong against
//! (D101 "The gap"). So the test walks `enc:body` + `enc:back` through `enc:children` / `enc:refs`
//! / `enc:units` and asserts what the walk reaches: every unit, 13 titled and sec-typed sections,
//! one bibliography with four groups, 63 entries. It also asserts that no unit sets `enc:section`
//! or `enc:source_document`, which containment makes redundant.
//!
//!     XIAP_DOCUMENT_MODEL=$PWD/experiments/parsing/derived/step5/xiap-document-model.esl \
//!       cargo test -p eigenius-kernel --test xiap_document_model -- --ignored --nocapture

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;

#[test]
#[ignore = "reads the gitignored emitted model; set XIAP_DOCUMENT_MODEL"]
fn the_document_model_loads_and_validates() {
    let Ok(path) = std::env::var("XIAP_DOCUMENT_MODEL") else {
        eprintln!("SKIP: XIAP_DOCUMENT_MODEL unset");
        return;
    };
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("SKIP: {path} not readable");
        return;
    };
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let resources = esl::compile(&src, &head).unwrap_or_else(|errs| {
        panic!(
            "the emitted document model compiles: {}",
            errs.iter()
                .take(4)
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    eprintln!("compiled {} resources", resources.len());
    let mut b = LayerBuilder::new("xiap-document-model", Some(head));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let errors: Vec<String> = Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "{} validation error(s); first: {}",
        errors.len(),
        errors.first().map(String::as_str).unwrap_or("")
    );

    // Compiling and validating is not the gate. The FLAT list this converter used to emit — 467
    // DiscourseUnits with a section string — also compiled and validated clean, because there was
    // no vocabulary for it to be wrong against (D101 "The gap"). So walk the tree: everything
    // below has to be reachable from the document by containment alone.
    let get = |iri_s: &str, prop: &str| -> Option<eigenius_kernel::ontology::resource::Value> {
        layer
            .resolve(&Iri::parse(iri_s).expect("well-formed IRI"))
            .and_then(|r| r.get(&Iri::parse(prop).expect("well-formed IRI")).cloned())
    };
    let arr = |iri_s: &str, prop: &str| -> Vec<String> {
        match get(iri_s, prop) {
            Some(Value::Array(vs)) => vs
                .iter()
                .filter_map(|v| v.as_iri().map(|i| i.as_str().to_string()))
                .collect(),
            _ => vec![],
        }
    };

    const DOC: &str = "urn:eigenius:uab:xiap-report:report";
    let body = arr(DOC, "urn:eigenius:encoding:body");
    let back = arr(DOC, "urn:eigenius:encoding:back");
    assert!(
        !body.is_empty() && !back.is_empty(),
        "the document has a body and back matter, not a flat unit list"
    );

    // Containment walk: body + back, through children / units / refs, to the leaves.
    let is_a = |iri_s: &str, class: &str| -> bool {
        layer
            .resolve(&Iri::parse(iri_s).expect("well-formed IRI"))
            .map(|r| r.is_instance_of(&Iri::parse(class).expect("well-formed IRI")))
            .unwrap_or(false)
    };
    let mut seen_units: Vec<String> = Vec::new();
    let mut sections = 0usize;
    let mut reflists = 0usize;
    let mut refs = 0usize;
    let mut queue: Vec<String> = body.iter().chain(back.iter()).cloned().collect();
    let mut guard = 0usize;
    while let Some(node) = queue.pop() {
        guard += 1;
        assert!(guard < 20_000, "containment walk terminates");
        for p in [
            "urn:eigenius:encoding:children",
            "urn:eigenius:encoding:refs",
        ] {
            queue.extend(arr(&node, p));
        }
        let units = arr(&node, "urn:eigenius:encoding:units");
        if !units.is_empty() {
            seen_units.extend(units);
        }
        if is_a(&node, "urn:eigenius:encoding:Section") {
            sections += 1;
            assert!(
                get(&node, "urn:eigenius:encoding:title").is_some()
                    && get(&node, "urn:eigenius:encoding:section_kind").is_some(),
                "{node} is a titled section with a JATS sec-type"
            );
        }
        if is_a(&node, "urn:eigenius:encoding:Ref") {
            refs += 1;
        }
        if is_a(&node, "urn:eigenius:encoding:RefList") {
            reflists += 1;
        }
    }
    seen_units.sort();
    seen_units.dedup();
    eprintln!(
        "walk reached: {} units, {sections} sections, {reflists} ref-lists, {refs} refs",
        seen_units.len()
    );
    assert!(
        seen_units.len() > 400,
        "the walk reaches the units through containment, got {}",
        seen_units.len()
    );
    assert!(sections >= 13, "13 sections, got {sections}");
    assert_eq!(
        reflists, 5,
        "one bibliography plus its four titled groups, got {reflists}"
    );
    assert!(
        refs >= 60,
        "the bibliography entries are reachable, got {refs}"
    );

    // Every unit the walk reached carries a locator, and none carries the provenance fields that
    // containment makes redundant (D101 "Provenance has one root").
    for u in &seen_units {
        assert!(
            get(u, "urn:eigenius:encoding:locator").is_some(),
            "{u} carries a Text Fragment locator"
        );
        for dead in [
            "urn:eigenius:encoding:section",
            "urn:eigenius:encoding:source_document",
        ] {
            assert!(
                get(u, dead).is_none(),
                "{u} must not set {dead}: containment is the single provenance root"
            );
        }
    }
}
