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

//! D65 §5 — the NCBI-Gene injector end to end: the rendered mirror + derived
//! lexicon (a) validate + felicity-gate against the real bootstrap stack, and
//! (b) drive a scoped parse of "WRN affects TP53" where the gene witnesses come
//! from the `ncbi_gene` lexicon and the verb from the demo, composing because
//! `ncbi:Gene ⊑ lexicon:Entity`.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{gate_entry, is_ctor, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;
use eigenius_ncbi_gene::convert::render_document;
use eigenius_ncbi_gene::gene_info::parse_document;

// Two real-shape human gene rows (GeneIDs 7486 / 7157).
const GENES: &str = "9606\t7486\tWRN\t-\tRECQ3|RECQL2\tHGNC:HGNC:12791|Ensembl:ENSG00000165392\t8\t8p12\tWerner syndrome RecQ like helicase\tprotein-coding\tWRN\tWerner syndrome RecQ like helicase\tO\t-\t20240101\t-
9606\t7157\tTP53\t-\tP53|LFS1\tHGNC:HGNC:11998\t17\t17p13.1\ttumor protein p53\tprotein-coding\tTP53\ttumor protein p53\tO\t-\t20240101\t-";

// The demo lexicon supplies the general transitive verb `affects` (Entity slots).
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
    let genes = parse_document(GENES, "9606");
    let (doc, rep) = render_document(&genes, "9606", false);
    assert_eq!(rep.genes, 2);

    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let ncbi = esl_layer("ncbi-gene", &doc, Arc::clone(ctx.head()));

    // Structural validation clean. We render with the WordNet anchor OFF here, so
    // ncbi:Gene roots at lexicon:Entity only — the base import validates standalone
    // (the wn:gene.n.01 grounding edge is opt-in, valid only on a WordNet chain).
    let errors = Validator::new(Arc::clone(&ncbi)).validate();
    assert!(errors.is_empty(), "structural errors: {errors:?}");

    // Every emitted lexical entry passes the felicity gate (⟦cat⟧ ≡ sem_type and
    // sem inhabits ⟦cat⟧).
    let entry_class = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    let mut gated = 0;
    for (id, r) in ncbi.iter_resources() {
        if r.is_instance_of(&entry_class) {
            gate_entry(&ncbi, &r).unwrap_or_else(|e| panic!("{id}: felicity gate rejected: {e}"));
            gated += 1;
        }
    }
    assert_eq!(gated, rep.entries, "all entries felicity-gated");
}

#[test]
fn scoped_parse_of_wrn_affects_tp53() {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    // bootstrap → demo (the `affects` verb) → ncbi-gene (WRN / TP53 witnesses).
    let demo = esl_layer("demo", DEMO, Arc::clone(ctx.head()));
    let genes = parse_document(GENES, "9606");
    let (doc, _) = render_document(&genes, "9606", false);
    let ncbi = esl_layer("ncbi-gene", &doc, demo);

    let index = Parser::build(Arc::clone(&ncbi));
    let ncbi_gene = Iri::parse("urn:eigenius:lexicon:ncbi_gene").unwrap();

    // Scoped to the gene lexicon: WRN / TP53 are in scope; `affects` and any
    // determiners are untagged ⇒ always available. The gene witnesses fill the
    // verb's Entity slots by subsumption (ncbi:Gene ⊑ Entity).
    let forest = index.parse_scoped(
        "WRN affects TP53",
        &Identity,
        Some(std::slice::from_ref(&ncbi_gene)),
    );
    assert!(
        !forest.is_empty(),
        "'WRN affects TP53' must parse with the ncbi_gene lexicon in scope"
    );
    assert!(
        forest.iter().all(|p| is_ctor(p.cat(), "cat_s").is_some()),
        "every parse is a sentence (S)"
    );

    // A gene symbol outside the scope and untagged-verb sanity: an out-of-scope
    // lexicon would drop the gene readings — here the only scope IS ncbi_gene, so
    // the genes resolve. (Cross-lexicon precedence is covered by the kernel slice-4
    // tests; this asserts the injector's entries are reachable under scope.)
}
