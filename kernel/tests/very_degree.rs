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
//! **Degree `very` over adjectival predicates** (`ontologies/lexicon/closed-class.esl`).
//!
//! Snapshot-free: reads the promoted entries in `closed-class.esl` through
//! `testing::bootstrap_context` + a minimal content-word fixture.
//!
//! Pins four properties:
//! 1. **Predicative position** («the gene is very essential») closes through the copula (`is_adj`)
//!    and conjoins the unmodified property with `very_deg(essential, …)` (Luo & Shi modifier drop).
//! 2. **Attributive position** («a very essential gene affects cells») reaches `cat_mod` through
//!    `UnaryKind::ModLift` on the composed two-token `S[dcl,adj]\NP` cell, needing no separate
//!    `cat_mod/cat_mod` entry.
//! 3. **Emphatic adjective coexistence** («the very gene affects cells»): `very`'s open-class
//!    adjective sense (`S[dcl,adj]\NP`) still refines a noun (`cat_n`), because the degree entry
//!    takes `S[dcl,adj]\NP` and is inert before a noun.
//! 4. **Over-generation refusals**: finite verbal VPs (*«the gene very affects cells») and base
//!    verbal VPs (*«the gene may very affect cells») are rejected because `very_adj` is typed at
//!    `adj` finiteness rather than `fin`/`bse` (unlike `adv_cats`, which binds `f : Fin` across
//!    verbal and adjectival clauses).

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Gene    : lexicon:Entity { }
class fixture:Protein : lexicon:Entity { }
class fixture:Cell    : lexicon:Entity { }

axiom fixture:essential : lexicon:Entity -> Prop
axiom fixture:very_emp  : lexicon:Entity -> Prop
axiom fixture:affects   : lexicon:Entity -> lexicon:Entity -> Prop

resource fixture:gene_n : lexicon:LexicalEntry {
    lexicon:form     = "gene";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Gene, lexicon:num_any) );
    lexicon:sem      = fixture:Gene;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_gene";
}
resource fixture:protein_n : lexicon:LexicalEntry {
    lexicon:form     = "protein";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Protein, lexicon:num_any) );
    lexicon:sem      = fixture:Protein;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_protein";
}
resource fixture:cells_n : lexicon:LexicalEntry {
    lexicon:form     = "cells";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Cell, lexicon:pl) );
    lexicon:sem      = fixture:Cell;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_cells";
}
resource fixture:essential_adj : lexicon:LexicalEntry {
    lexicon:form     = "essential";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:essential;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_essential";
}
// Open-class emphatic adjective `very` ("the very center", "this very codon"), modelled alongside
// the degree modifier to pin that the two do not collide.
resource fixture:very_emphatic_adj : lexicon:LexicalEntry {
    lexicon:form     = "very";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:very_emp;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "wn:very.a.01839317";
}
resource fixture:affects_v : lexicon:LexicalEntry {
    lexicon:form     = "affects";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:affects;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_affects";
}
resource fixture:affect_bse : lexicon:LexicalEntry {
    lexicon:form     = "affect";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:bse), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:affects;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_affects";
}
"#;

fn parser() -> Parser {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("very-degree", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("source compiles") {
        b.add_resource(r).expect("add resource");
    }
    let layer: Arc<Layer> = Arc::new(b.build(LayerStorage::in_memory()));
    Parser::build(layer)
}

fn sems(sentence: &str) -> Vec<String> {
    let (closed, _open) = parser().parse_open(sentence, &Identity);
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

/// Predicative `very` («The gene is very essential.» — battery line 38) closes through `is_adj`
/// and produces the conjoined `And(essential(the(Gene)), very_deg(essential, the(Gene)))`.
#[test]
fn predicative_very_conjoins_the_adjective_and_the_degree_modifier() {
    let got = sems("the gene is very essential");
    assert!(
        got.iter().any(|s| {
            s.contains("And(")
                && s.contains("essential(the(Gene))")
                && s.contains("very_deg(")
        }),
        "predicative `very essential` must conjoin essential(the(Gene)) and very_deg(essential, the(Gene)); got {got:?}"
    );
}

/// Attributive `very` («A very essential gene affects cells.») reaches `cat_mod` via `ModLift` on
/// the composed two-token `S[dcl,adj]\NP` cell, refining `Gene` without a second lexical entry.
#[test]
fn attributive_very_lifts_composed_adjective_to_cat_mod() {
    let got = sems("a very essential gene affects cells");
    assert!(
        got.iter().any(|s| {
            s.contains("Gene")
                && s.contains("essential(")
                && s.contains("very_deg(")
                && s.contains("affects(")
        }),
        "attributive `very essential gene` must refine Gene with both essential and very_deg; got {got:?}"
    );
}

/// Emphatic adjective `very` («the very gene affects cells») still refines a noun directly when
/// no adjective follows `very`.
#[test]
fn emphatic_adjective_very_before_a_noun_is_untouched() {
    let got = sems("the very gene affects cells");
    assert!(
        got.iter()
            .any(|s| s.contains("very_emp(") && !s.contains("very_deg(")),
        "emphatic adjective `very` before a noun must still parse via very_emp; got {got:?}"
    );
}

/// `very` must NOT modify a finite verbal VP (*«the gene very affects cells») or a base verbal VP
/// (*«the gene may very affect cells»).
#[test]
fn very_refuses_finite_and_base_verbal_vps() {
    assert!(
        !sems("the gene may affect cells").is_empty(),
        "control: `the gene may affect cells` must parse"
    );
    let verbal_fin = sems("the gene very affects cells");
    assert!(
        verbal_fin.is_empty(),
        "`very` must not modify a finite verb; got {verbal_fin:?}"
    );
    let verbal_bse = sems("the gene may very affect cells");
    assert!(
        verbal_bse.is_empty(),
        "`very` must not modify a base verb under a modal; got {verbal_bse:?}"
    );
}
