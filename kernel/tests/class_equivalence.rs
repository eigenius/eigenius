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

//! `core:EquivalentClasses` (D99 §11) on the real bootstrap chain.
//!
//! - **Subsumption.** Three classes for one concept, declared independently, are each other's
//!   subclasses once an equivalence lists them; a class it does not list stays unrelated.
//! - **Inhabitation.** An instance of the WordNet class inhabits a predicate typed over the HPO
//!   class.
//! - **Rule 26.** An equivalence of classes with different obligations, or of one class, is
//!   refused.
//! - **No cycle.** The equivalence layer passes declaration ordering, though its effect on
//!   subsumption is mutual.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::validation::Validator;

fn compile_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile(src, &parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile: {}",
            errs.iter()
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// bootstrap → the classes → the equivalences.
fn chain() -> (Arc<Layer>, Arc<Layer>) {
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let classes = compile_layer(
        "classes",
        include_str!("fixtures/class_equivalence.esl"),
        head,
    );
    let link = compile_layer(
        "link",
        include_str!("fixtures/class_equivalence_link.esl"),
        Arc::clone(&classes),
    );
    (classes, link)
}

fn iri(local: &str) -> Iri {
    Iri::parse(&format!("urn:eigenius:test:equiv:{local}")).unwrap()
}

fn errors(layer: &Arc<Layer>, local: &str) -> Vec<String> {
    let target = iri(local);
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref() == Some(&target))
        .map(|e| format!("{e:?}"))
        .collect()
}

#[test]
fn listed_classes_subsume_each_other() {
    let (classes, link) = chain();
    let (hp, wn, umls) = (iri("HP_0000103"), iri("wn_polyuria"), iri("C0032617"));
    assert!(
        !classes.is_subclass_of(&wn, &hp),
        "unrelated before the link"
    );
    for (a, b) in [(&hp, &wn), (&wn, &hp), (&umls, &hp), (&wn, &umls)] {
        assert!(link.is_subclass_of(a, b), "{a} ⊑ {b}");
    }
    assert!(!link.is_subclass_of(&wn, &iri("HP_0001959")));
}

#[test]
fn an_instance_of_one_inhabits_a_predicate_over_another() {
    let (_, link) = chain();
    assert_eq!(errors(&link, "observed_episode"), Vec::<String>::new());
}

#[test]
fn the_equivalence_layer_is_not_a_declaration_cycle() {
    let (_, link) = chain();
    assert_eq!(errors(&link, "polyuria"), Vec::<String>::new());
}

#[test]
fn classes_with_different_obligations_are_not_equivalent() {
    let (_, link) = chain();
    let errs = errors(&link, "unequal");
    assert!(
        errs.iter()
            .any(|e| e.contains("EquivalenceUnsound") && e.contains("require different")),
        "{errs:?}"
    );
}

#[test]
fn one_class_is_not_an_equivalence() {
    let (_, link) = chain();
    let errs = errors(&link, "lonely");
    assert!(
        errs.iter()
            .any(|e| e.contains("EquivalenceUnsound") && e.contains("at least two")),
        "{errs:?}"
    );
}

/// The control: the same declaration, on the chain without the equivalence, is ill-typed.
#[test]
fn without_the_link_the_instance_does_not_inhabit_the_other_class() {
    let (classes, _) = chain();
    let unlinked = compile_layer(
        "unlinked",
        r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace prov          = "urn:eigenius:prov";
namespace fx            = "urn:eigenius:test:equiv";
resource fx:observed_episode : justification:Declaration {
    core:description = "The episode is an observed HPO polyuria.";
    prov:was_attributed_to = fx:author;
    eigentt:proposition = type_expr(fx:Observed(fx:episode));
}
"#,
        classes,
    );
    let errs = errors(&unlinked, "observed_episode");
    assert!(
        errs.iter().any(|e| e.contains("does not inhabit class")),
        "{errs:?}"
    );
}
