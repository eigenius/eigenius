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

//! **Determiner free relatives / embedded `which`-clauses** (`experiments/lexicon/embedded-wh.esl`)
//! — snapshot-free integration test.
//!
//! Pins:
//! 1. **PP-complement subject-gap `which`-clause** («nothing turns on which cutoff is used»):
//!    closes as a declarative with `the(Σx:Cutoff. …).1` as the preposition's object.
//! 2. **Object-gap `which`-clause** («the study reported which cutoff the assay used»):
//!    closes as a declarative with `the(Σx:Cutoff. used(x, the(Assay))).1`.
//! 3. **Root `which`-question isolation** («which cutoff is used»): still closes only as
//!    the root question's answer-property `λx:Cutoff. …` (`cat_q(Cutoff)`), not as a statement.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};

const LAYER: &str = include_str!("../../experiments/lexicon/embedded-wh.esl");

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Cutoff : lexicon:Entity { }
class fixture:Study  : lexicon:Entity { }
class fixture:Assay  : lexicon:Entity { }

axiom fixture:turns : lexicon:Entity -> Prop
axiom fixture:used  : lexicon:Entity -> lexicon:Entity -> Prop
axiom fixture:reported : lexicon:Entity -> lexicon:Entity -> Prop

resource fixture:cutoff_n : lexicon:LexicalEntry {
    lexicon:form     = "cutoff";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Cutoff, lexicon:sg) );
    lexicon:sem      = fixture:Cutoff;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_cutoff";
}
resource fixture:study_n : lexicon:LexicalEntry {
    lexicon:form     = "study";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Study, lexicon:sg) );
    lexicon:sem      = fixture:Study;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_study";
}
resource fixture:assay_n : lexicon:LexicalEntry {
    lexicon:form     = "assay";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Assay, lexicon:sg) );
    lexicon:sem      = fixture:Assay;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_assay";
}
resource fixture:turns_v : lexicon:LexicalEntry {
    lexicon:form     = "turns";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)) );
    lexicon:sem      = fixture:turns;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_turns";
}
resource fixture:used_pss : lexicon:LexicalEntry {
    lexicon:form     = "used";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:pss), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:used;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_used";
}
resource fixture:used_fin : lexicon:LexicalEntry {
    lexicon:form     = "used";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:used;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_used";
}
resource fixture:reported_v : lexicon:LexicalEntry {
    lexicon:form     = "reported";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:reported;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_reported";
}
"#;

fn parser() -> Parser {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut head = Arc::clone(ctx.head());
    for (name, src) in [
        ("embedded-wh-layer", LAYER),
        ("embedded-wh-fixture", FIXTURE),
    ] {
        let mut b = LayerBuilder::new(name, Some(Arc::clone(&head)));
        for r in esl::compile(src, &head).expect("source compiles") {
            b.add_resource(r).expect("add resource");
        }
        head = Arc::new(b.build(LayerStorage::in_memory()));
    }
    Parser::build(head)
}

fn sems(sentence: &str) -> Vec<String> {
    let (closed, _open) = parser().parse_open(sentence, &Identity);
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

#[test]
fn pp_complement_subject_gap_which_clause_closes_as_definite() {
    let got = sems("nothing turns on which cutoff is used");
    assert!(
        got.iter().any(|s| {
            s.contains("adv_on(the(Σ")
                && s.contains(":Cutoff.")
                && s.contains("used(")
                && s.contains(").1")
        }),
        "expected `nothing turns on which cutoff is used` to close with `adv_on(the(Σ...:Cutoff. ... used(...)).1, ...)`, got {got:?}"
    );
}

#[test]
fn object_gap_which_clause_closes_as_definite() {
    let got = sems("the study reported which cutoff the assay used");
    assert!(
        got.iter().any(|s| {
            s.contains("reported(the(Σ")
                && s.contains(":Cutoff.")
                && s.contains("used(")
                && s.contains("the(Assay)")
                && s.contains(").1")
        }),
        "expected `the study reported which cutoff the assay used` to close with `reported(the(Σ...:Cutoff. used(..., the(Assay))).1, the(Study))`, got {got:?}"
    );
}

#[test]
fn bare_which_clause_still_closes_only_as_a_question() {
    let got = sems("which cutoff is used");
    assert_eq!(
        got.len(),
        1,
        "bare `which cutoff is used` must keep its single root-question reading; got {got:?}"
    );
    assert!(
        got[0].starts_with('λ') && !got[0].contains("the(Σ"),
        "bare `which cutoff is used` must denote the answer-property, not a definite description; got {got:?}"
    );
}
