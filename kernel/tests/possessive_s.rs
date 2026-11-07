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

//! **Possessive clitic `'s`** (`det.xsl` `PossS`) — snapshot-free integration test.
//!
//! Verifies that `'s` attaches to a preceding `cat_np` (both a determined common noun
//! `the study` and a proper noun `HeLa`), yields a `cat_forall` determiner over
//! `lexicon:poss_of`, and works in subject and object positions.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Study : lexicon:Entity { }
class fixture:Gene  : lexicon:Entity { }

axiom fixture:hela      : lexicon:Entity
axiom fixture:essential : lexicon:Entity -> Prop
axiom fixture:affects   : lexicon:Entity -> lexicon:Entity -> Prop

resource fixture:study_n : lexicon:LexicalEntry {
    lexicon:form     = "study";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Study, lexicon:sg) );
    lexicon:sem      = fixture:Study;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_study";
}
resource fixture:gene_n : lexicon:LexicalEntry {
    lexicon:form     = "gene";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Gene, lexicon:sg) );
    lexicon:sem      = fixture:Gene;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_gene";
}
resource fixture:genes_n : lexicon:LexicalEntry {
    lexicon:form     = "genes";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Gene, lexicon:pl) );
    lexicon:sem      = fixture:Gene;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_genes";
}
resource fixture:hela_np : lexicon:LexicalEntry {
    lexicon:form     = "hela";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem      = fixture:hela;
    lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense    = "fixture_hela";
}
resource fixture:essential_adj : lexicon:LexicalEntry {
    lexicon:form     = "essential";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:essential;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_essential";
}
resource fixture:affects_v : lexicon:LexicalEntry {
    lexicon:form     = "affects";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:affects;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_affects";
}
"#;

fn parser() -> Parser {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("possessive-s", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("source compiles") {
        b.add_resource(r).expect("add resource");
    }
    let layer: Arc<Layer> = Arc::new(b.build(LayerStorage::in_memory()));
    Parser::build(layer)
}

fn sems(sentence: &str) -> Vec<String> {
    let (closed, _open) = parser().parse_open(sentence, &Identity);
    closed.iter().map(|it| pretty_term(it.sem())).collect()
}

#[test]
fn determined_common_noun_possessor_in_subject_position() {
    let readings = sems("the study's gene is essential");
    assert!(
        readings.iter().any(|s| {
            s.contains("poss_of(Gene, G#1, the(Study))") && s.contains("essential(G#1)")
        }),
        "expected `poss_of(Gene, G#1, the(Study))` ∧ `essential(G#1)`; got: {readings:#?}"
    );
}

#[test]
fn plural_possessed_head_agrees_with_plural_verb() {
    let readings = sems("the study's genes are essential");
    assert!(
        readings.iter().any(|s| {
            s.contains("poss_of(Gene, G#1, the(Study))") && s.contains("essential(G#1)")
        }),
        "expected plural head `genes` to parse with `poss_of`; got: {readings:#?}"
    );
}

#[test]
fn proper_noun_possessor_and_object_position() {
    let subj_readings = sems("HeLa's gene is essential");
    assert!(
        subj_readings
            .iter()
            .any(|s| s.contains("poss_of(Gene, G#1, hela)") && s.contains("essential(G#1)")),
        "expected proper noun possessor `HeLa's gene`; got: {subj_readings:#?}"
    );

    let obj_readings = sems("HeLa affects the study's gene");
    assert!(
        obj_readings.iter().any(|s| {
            s.contains("poss_of(Gene, G#1, the(Study))") && s.contains("affects(G#1, hela)")
        }),
        "expected object-position `the study's gene`; got: {obj_readings:#?}"
    );
}
