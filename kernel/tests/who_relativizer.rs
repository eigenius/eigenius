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
//! **`who` as a relativizer** — one reserved-construct resource, no Rust. Snapshot-free: the
//! resource under test is the PROMOTED `lexicon:rc_who` in `ontologies/lexicon/closed-class.esl`,
//! reached through `testing::bootstrap_context`. Reading it from the bootstrap rather than from a
//! probe layer is the point: a copy would keep these tests green even if the promotion were wrong.
//!
//! What the tests pin is that `who` behaves EXACTLY as the already-declared `that` does — same
//! `relativize` route, same `Σ` restrictor — and that the `that`-control still works, so a failure
//! here distinguishes "the new resource is wrong" from "the fixture cannot build a relative clause
//! at all".

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Reviewer : lexicon:Entity { }

axiom lexicon:distrusts : lexicon:Entity -> lexicon:Entity -> Prop
axiom lexicon:found     : lexicon:Entity -> lexicon:Entity -> Prop
axiom lexicon:evidence  : lexicon:Entity
axiom lexicon:study     : lexicon:Entity

resource lexicon:wr_reviewer : lexicon:LexicalEntry {
    lexicon:form     = "reviewer";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Reviewer, lexicon:sg) );
    lexicon:sem      = fixture:Reviewer;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "wr_reviewer";
}
resource lexicon:wr_evidence : lexicon:LexicalEntry {
    lexicon:form     = "evidence"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem = lexicon:evidence; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "wr_evidence";
}
resource lexicon:wr_study : lexicon:LexicalEntry {
    lexicon:form     = "study"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem = lexicon:study; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "wr_study";
}
// 3sg finite transitives — singular subject slot, `num_any` object.
resource lexicon:wr_distrusts : lexicon:LexicalEntry {
    lexicon:form     = "distrusts";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:distrusts;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wr_distrusts";
}
resource lexicon:wr_found : lexicon:LexicalEntry {
    lexicon:form     = "found";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:found;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wr_found";
}
"#;

fn sems(sentence: &str) -> Vec<String> {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("who-rel", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("fixture compiles") {
        b.add_resource(r).expect("add resource");
    }
    let layer: Arc<Layer> = Arc::new(b.build(LayerStorage::in_memory()));
    let (closed, _open) = Parser::build(layer).parse_open(sentence, &Identity);
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

/// The CONTROL: `that` is already declared, so a subject relative on it must parse. If this fails,
/// the fixture cannot build a relative clause and the `who` result below says nothing.
#[test]
fn the_declared_that_relativizer_builds_a_subject_relative() {
    let got = sems("study found the reviewer that distrusts evidence");
    assert!(
        got.iter()
            .any(|s| s.contains("distrusts(evidence") && s.contains("Reviewer")),
        "`that` must build Σx:Reviewer. distrusts(evidence, x); got {got:?}"
    );
}

/// `who` reaches the SAME route. One resource, and the form is now a relativizer.
#[test]
fn who_builds_the_same_subject_relative_as_that() {
    let got = sems("study found the reviewer who distrusts evidence");
    assert!(
        got.iter()
            .any(|s| s.contains("distrusts(evidence") && s.contains("Reviewer")),
        "`who` must build Σx:Reviewer. distrusts(evidence, x); got {got:?}"
    );
}

/// The two forms are INTERCHANGEABLE here: a relativizer contributes nothing to the claim, so the
/// `who` and `that` readings must be the same set of terms. This is the real content of the change
/// — not that `who` parses, but that it parses to what `that` already did.
#[test]
fn who_and_that_yield_the_same_readings() {
    let mut with_who = sems("study found the reviewer who distrusts evidence");
    let mut with_that = sems("study found the reviewer that distrusts evidence");
    with_who.sort();
    with_that.sort();
    assert_eq!(
        with_who, with_that,
        "a relativizer is not part of the claim, so both forms must give the same readings"
    );
}
