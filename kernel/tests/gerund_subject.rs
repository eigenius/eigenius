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

//! **Gerund VP subjects** (`kernel/src/dcg/rules/constructions.rs::gerund_subject`) —
//! snapshot-free integration test.
//!
//! Pins:
//! 1. **Gerund VP subject with singular copular predicate** («Reporting genes is essential.»):
//!    closes as a declarative over `kind_of(Σs:Entity. report(kind_of(Gene), s))`.
//! 2. **Gerund VP subject with modal VP** («Reporting the gene should restore the cell.»):
//!    closes as a declarative over `kind_of(Σs:Entity. report(the(Gene), s))`.
//! 3. **Plural agreement rejection** («*Reporting genes are essential.»):
//!    refused because a nominalized gerund VP subject is grammatically singular (`sg`).

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Gene : lexicon:Entity { }
class fixture:Cell : lexicon:Entity { }

axiom fixture:report    : lexicon:Entity -> lexicon:Entity -> Prop
axiom fixture:restore   : lexicon:Entity -> lexicon:Entity -> Prop
axiom fixture:essential : lexicon:Entity -> Prop

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
resource fixture:cell_sg : lexicon:LexicalEntry {
    lexicon:form     = "cell";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Cell, lexicon:sg) );
    lexicon:sem      = fixture:Cell;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_cell";
}
resource fixture:reporting_ger : lexicon:LexicalEntry {
    lexicon:form     = "reporting";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:ger), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:report;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_report";
}
resource fixture:restore_bse : lexicon:LexicalEntry {
    lexicon:form     = "restore";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:bse), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:restore;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_restore";
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
    let mut b = LayerBuilder::new("gerund-subject-fixture", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("source compiles") {
        b.add_resource(r).expect("add resource");
    }
    Parser::build(Arc::new(b.build(LayerStorage::in_memory())))
}

#[test]
fn gerund_subject_with_singular_copula_parses() {
    let p = parser();
    let (closed, open) = p.parse_open("Reporting genes is essential.", &Identity);
    let closed: Vec<String> = closed.iter().map(|i| pretty_term(i.sem())).collect();
    assert!(
        !closed.is_empty(),
        "expected a closed parse for 'Reporting genes is essential.'; open={:?}",
        open.iter()
            .map(|o| pretty_term(o.item.sem()))
            .collect::<Vec<_>>()
    );
    assert!(
        closed
            .iter()
            .any(|s| s.contains("essential") && s.contains("kind_of") && s.contains("report")),
        "expected nominalized gerund subject in sem; got: {closed:#?}"
    );
}

#[test]
fn gerund_subject_with_modal_vp_parses() {
    let p = parser();
    let (closed, _open) = p.parse_open("Reporting the gene should restore the cell.", &Identity);
    let closed: Vec<String> = closed.iter().map(|i| pretty_term(i.sem())).collect();
    assert!(
        !closed.is_empty(),
        "expected a closed parse for 'Reporting the gene should restore the cell.'; got none"
    );
    assert!(
        closed.iter().any(|s| {
            s.contains("Should")
                && s.contains("restore")
                && s.contains("kind_of")
                && s.contains("report")
        }),
        "expected modal over nominalized gerund subject; got: {closed:#?}"
    );
}

#[test]
fn gerund_subject_refuses_plural_verb_agreement() {
    let p = parser();
    let (closed, open) = p.parse_open("Reporting genes are essential.", &Identity);
    assert!(
        closed.is_empty() && open.is_empty(),
        "gerund VP subject is singular and must refuse plural 'are'; got closed={:?}",
        closed
            .iter()
            .map(|i| pretty_term(i.sem()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn preposition_with_gerund_vp_and_fronted_pp_parse() {
    let p = parser();
    let (closed, open) = p.parse_open("Without reporting genes, the cell is essential.", &Identity);
    let closed: Vec<String> = closed.iter().map(|i| pretty_term(i.sem())).collect();
    assert!(
        !closed.is_empty(),
        "expected a closed parse for 'Without reporting genes, the cell is essential.'; open={:?}",
        open.iter()
            .map(|o| pretty_term(o.item.sem()))
            .collect::<Vec<_>>()
    );
    assert!(
        closed.iter().any(|s| {
            s.contains("adv_without")
                && s.contains("kind_of")
                && s.contains("report")
                && s.contains("essential")
        }),
        "expected fronted 'Without reporting genes,' to attach to the matrix clause; got: {closed:#?}"
    );

    let (closed_by, _open_by) =
        p.parse_open("By reporting genes, the cell is essential.", &Identity);
    let closed_by: Vec<String> = closed_by.iter().map(|i| pretty_term(i.sem())).collect();
    assert!(
        closed_by.iter().any(|s| {
            s.contains("prep_by")
                && s.contains("kind_of")
                && s.contains("report")
                && s.contains("essential")
        }),
        "expected fronted 'By reporting genes,' to attach to the matrix clause; got: {closed_by:#?}"
    );

    let (closed_det, _open_det) = p.parse_open("In a cell, the gene is essential.", &Identity);
    let closed_det: Vec<String> = closed_det.iter().map(|i| pretty_term(i.sem())).collect();
    assert!(
        closed_det
            .iter()
            .any(|s| s.contains("adv_in") && s.contains("essential(the(Gene))")),
        "expected fronted PP with quantified object 'In a cell,' to parse; got: {closed_det:#?}"
    );
}
