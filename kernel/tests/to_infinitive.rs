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

//! **`to`-infinitive subject control and ECM** (`ontologies/lexicon/closed-class.esl`
//! `to_inf`, `to_ecm`, `be_copula`) — snapshot-free integration test.
//!
//! Pins:
//! 1. **Subject control (`The study wants to report genes.`)**:
//!    `to_inf` lifts `report genes` (`S[dcl,bse]\NP`) to `S[dcl,inf]\NP`, consumed by
//!    `wants` (`(S[dcl,fin]\NP_sg)/(S[dcl,inf]\NP)`).
//! 2. **ECM / subject-to-object raising (`The study showed the gene to be essential.`)**:
//!    `be_copula` + `essential` forms `be essential` (`S[dcl,bse]\NP`), `to_ecm` lifts it to
//!    `cat_cp \⋆ NP`, `the gene` backward-applies to yield `cat_cp` (`essential(the(Gene))`),
//!    and `showed` (`(S[dcl,fin]\NP)/cat_cp`) consumes the embedded proposition.
//! 3. **Non-root guard (`*The gene to be essential.`)**:
//!    refused as a matrix clause because `cat_cp` is not a root sentence category.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Lemmatizer, Parser, Pos};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};

struct TestLemmatizer;

impl Lemmatizer for TestLemmatizer {
    fn lemmas(&self, form: &str, pos: Pos) -> Vec<String> {
        match (form, pos) {
            ("showed", Pos::Verb) => vec!["show".to_string()],
            _ => vec![form.to_string()],
        }
    }
}

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Study : lexicon:Entity { }
class fixture:Gene  : lexicon:Entity { }

axiom fixture:wants     : (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop
axiom fixture:report    : lexicon:Entity -> lexicon:Entity -> Prop
axiom fixture:showed    : Prop -> lexicon:Entity -> Prop
axiom fixture:essential : lexicon:Entity -> Prop

resource fixture:study_sg : lexicon:LexicalEntry {
    lexicon:form     = "study";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Study, lexicon:sg) );
    lexicon:sem      = fixture:Study;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_study";
}
resource fixture:gene_sg : lexicon:LexicalEntry {
    lexicon:form     = "gene";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Gene, lexicon:sg) );
    lexicon:sem      = fixture:Gene;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_gene";
}
resource fixture:genes_pl : lexicon:LexicalEntry {
    lexicon:form     = "genes";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Gene, lexicon:pl) );
    lexicon:sem      = fixture:Gene;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_gene";
}
resource fixture:wants_sc : lexicon:LexicalEntry {
    lexicon:form     = "wants";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:inf), lexicon:cat_np(lexicon:Entity, lexicon:num_any))) );
    lexicon:sem      = fixture:wants;
    lexicon:sem_type = type_expr( (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_wants";
}
resource fixture:report_bse : lexicon:LexicalEntry {
    lexicon:form     = "report";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:bse), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:report;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_report";
}
resource fixture:show_c_fp : lexicon:LexicalEntry {
    lexicon:form     = "show";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:pl)), lexicon:cat_cp) );
    lexicon:sem      = fixture:showed;
    lexicon:sem_type = type_expr( Prop -> lexicon:Entity -> Prop );
    lexicon:sense    = "wn:show.v.01012244";
}
resource fixture:essential_adj : lexicon:LexicalEntry {
    lexicon:form     = "essential";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:essential;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_essential";
}
"#;

fn parser() -> Parser {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("to-infinitive-fixture", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("source compiles") {
        b.add_resource(r).expect("add resource");
    }
    Parser::build(Arc::new(b.build(LayerStorage::in_memory())))
}

fn sems(sentence: &str) -> Vec<String> {
    let (closed, _open) = parser().parse_open(sentence, &TestLemmatizer);
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

#[test]
fn subject_control_wants_to_report_genes_parses() {
    let got = sems("The study wants to report genes.");
    assert!(
        !got.is_empty(),
        "expected a closed parse for 'The study wants to report genes.'; got none"
    );
    assert!(
        got.iter().any(|s| {
            s.contains("wants(")
                && s.contains("report(kind_of(Gene)")
                && s.contains("the(Study)")
        }),
        "expected wants(λx. report(kind_of(Gene), x), the(Study)); got: {got:#?}"
    );
}

#[test]
fn ecm_showed_the_gene_to_be_essential_parses() {
    let got = sems("The study showed the gene to be essential.");
    assert!(
        !got.is_empty(),
        "expected a closed parse for 'The study showed the gene to be essential.'; got none"
    );
    assert!(
        got.iter().any(|s| {
            s.contains("showed(essential(the(Gene)), the(Study))")
        }),
        "expected showed(essential(the(Gene)), the(Study)); got: {got:#?}"
    );
}

#[test]
fn unembedded_ecm_infinitive_cannot_stand_as_root_clause() {
    assert!(
        sems("The gene to be essential.").is_empty(),
        "unembedded 'The gene to be essential.' is a cat_cp, not a root declarative clause"
    );
}

#[test]
fn base_show_rejects_singular_subject_while_showed_accepts_it() {
    assert!(
        sems("The study show the gene to be essential.").is_empty(),
        "base finite 'show' carries a plural subject slot and must reject singular 'The study'"
    );
}

