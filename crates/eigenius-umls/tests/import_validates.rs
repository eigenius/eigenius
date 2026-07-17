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

//! D65 §5 — the UMLS injector end to end: the rendered mirror + derived lexicon
//! (a) validate + felicity-gate against the real bootstrap stack, and (b) drive a
//! scoped parse of "every Werner syndrome affects HeLa" where the disease KIND comes
//! from the `umls` lexicon (a common-noun `cat_n`), composing because
//! `umlscui:C0043119 ⊑ umlssty:T047 ⊑ lexicon:Entity`.
//!
//! Fixtures are real-shape rows for Werner syndrome (C0043119, T047 Disease or
//! Syndrome) and Microsatellite Instability (C0920269, T049), plus a restricted
//! SNOMED CT source that the SRL-0 filter must drop.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{gate_entry, is_ctor, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;
use eigenius_umls::convert::render_document;
use eigenius_umls::rrf::build_subset;

// RRF MRSAB: RSAB is col 4 (index 3), SRL is col 14 (index 13).
const MRSAB: &str = "C1|C1|MSH2026|MSH|MeSH|MSH|2026|||||||0|1|1|FULL|MH||ENG|UTF-8|Y|Y|MeSH|;|
C2|C2|NCI2026|NCI|NCI Thesaurus|NCI|2026|||||||0|1|1|FULL|PT||ENG|UTF-8|Y|Y|NCI|;|
C9|C9|SNOMEDCT_US_2026|SNOMEDCT_US|SNOMED CT|SNOMEDCT_US|2026|||||||9|1|1|FULL|PT||ENG|UTF-8|Y|Y|SNOMEDCT|;|";

const MRRANK: &str = "0500|MSH|MH|N|
0490|NCI|PT|N|
0100|SNOMEDCT_US|PT|N|";

const MRSTY: &str = "C0043119|T047|B2.2.1.2.1|Disease or Syndrome|AT1||
C0920269|T049|A1.2.2.2|Cell or Molecular Dysfunction|AT2||";

const MRCONSO: &str = "C0043119|ENG|P|L1|PF|S1|Y|A1||||MSH|MH|D014898|Werner Syndrome|0|N||
C0043119|ENG|S|L2|VO|S2|N|A2||||NCI|SY|C1|Werner's Syndrome|0|N||
C0043119|ENG|S|L3|VO|S3|N|A3||||SNOMEDCT_US|PT|111|Werner syndrome (disorder)|0|N||
C0920269|ENG|P|L6|PF|S6|Y|A6||||MSH|MH|D053842|Microsatellite Instability|0|N||";

const MRDEF: &str = "C0043119|A1|AT10||MSH|An autosomal recessive disorder of premature aging.|N||
C0043119|A2|AT11||NCI|A rare syndrome caused by WRN mutations.|N||";

// The demo lexicon supplies the general transitive verb `affects` (Entity slots) and
// the proper noun `HeLa`.
const DEMO: &str = include_str!("../../../experiments/lexicon/lexicon.esl");

/// Compile `doc` against `parent` and build the layer (panicking with the compile
/// errors if it isn't Expressible).
fn esl_layer(name: &str, doc: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile_against_layer(doc, &parent)
        .unwrap_or_else(|e| panic!("{name} failed to compile: {e:?}"));
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r).expect("add_resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn mirror_and_lexicon_validate_and_felicity_gate() {
    let subset = build_subset(MRSAB, MRRANK, MRSTY, MRCONSO, MRDEF, None, "ENG", None);
    let (doc, rep) = render_document(&subset, "2026AA", &Default::default(), &Default::default());
    assert_eq!(rep.concepts, 2);
    assert_eq!(rep.semantic_types, 2);

    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let umls = esl_layer("umls", &doc, Arc::clone(ctx.head()));

    // Structural validation clean. Concept classes root at lexicon:Entity transitively
    // (umlscui:C ⊑ umlssty:T ⊑ lexicon:Entity), so the import validates standalone on
    // bootstrap — no WordNet anchor required.
    let errors = Validator::new(Arc::clone(&umls)).validate();
    assert!(errors.is_empty(), "structural errors: {errors:?}");

    // Every emitted lexical entry passes the felicity gate (⟦cat_n⟧ ≡ sem_type = Set,
    // and the concept-class sem inhabits Set).
    let entry_class = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    let mut gated = 0;
    for (id, r) in umls.iter_resources() {
        if r.is_instance_of(&entry_class) {
            gate_entry(&umls, &r).unwrap_or_else(|e| panic!("{id}: felicity gate rejected: {e}"));
            gated += 1;
        }
    }
    assert_eq!(gated, rep.entries, "all entries felicity-gated");
}

#[test]
fn scoped_parse_of_every_werner_syndrome_affects_hela() {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    // bootstrap → demo (the `affects` verb + HeLa) → umls (Werner syndrome kind).
    let demo = esl_layer("demo", DEMO, Arc::clone(ctx.head()));
    let subset = build_subset(MRSAB, MRRANK, MRSTY, MRCONSO, MRDEF, None, "ENG", None);
    let (doc, _) = render_document(&subset, "2026AA", &Default::default(), &Default::default());
    let umls = esl_layer("umls", &doc, demo);

    let index = Parser::build(Arc::clone(&umls));
    let umls_lex = Iri::parse("urn:eigenius:lexicon:umls").unwrap();

    // Scoped to the UMLS lexicon: the disease KIND "Werner syndrome" is in scope; the
    // closed-class determiner `every` and the demo verb `affects` / `HeLa` are untagged
    // ⇒ always available. `every` quantifies the cat_n kind; the bound variable fills
    // the verb's Entity slot by subsumption (umlscui:C0043119 ⊑ … ⊑ Entity).
    let forest = index.parse_scoped(
        "every Werner syndrome affects HeLa",
        &Identity,
        Some(std::slice::from_ref(&umls_lex)),
    );
    assert!(
        !forest.is_empty(),
        "'every Werner syndrome affects HeLa' must parse with the umls lexicon in scope"
    );
    assert!(
        forest.iter().all(|p| is_ctor(p.cat(), "cat_s").is_some()),
        "every parse is a sentence (S)"
    );
}
