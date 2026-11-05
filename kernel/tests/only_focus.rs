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

//! **Pre-nominal NP focus `only`** (`experiments/lexicon/only-focus.esl`) — snapshot-free integration test.
//!
//! Pins:
//! 1. **Subject indefinite / cardinal / definite NP focus** («only an assay resolves it»,
//!    «only one pattern is safe», «only the gene is essential») restricts the witness with
//!    `ontology:sole`, matching post-nominal `alone`.
//! 2. **Object NP focus** («the study involves only four genes») closes in object position
//!    with `ontology:sole`.
//! 3. **Gate on `adv_initial_lifts`**: when `only` carries an open-class manner-adverb sense
//!    alongside `only_focus_subj`, sentence-initial `only` does NOT double-seed the manner-adverb
//!    reading via `adv_initial_lifts`.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const LAYER: &str = include_str!("../../experiments/lexicon/only-focus.esl");

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace logic   = "urn:eigenius:logic";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Assay   : lexicon:Entity { }
class fixture:Pattern : lexicon:Entity { }
class fixture:Gene    : lexicon:Entity { }
class fixture:Study   : lexicon:Entity { }

axiom fixture:safe      : lexicon:Entity -> Prop
axiom fixture:essential : lexicon:Entity -> Prop
axiom fixture:involves  : lexicon:Entity -> lexicon:Entity -> Prop
axiom fixture:only_adv  : (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop

resource fixture:only_adv_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (V : lexicon:Entity -> Prop) => fun (s : lexicon:Entity) =>
            logic:And(V(s), fixture:only_adv(V, s))
          : (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop) )
    );
}
// Open-class manner-adverb entry for `only` (as WordNet imports `r00004722`), to verify
// that `only_focus_subj` suppresses `adv_initial_lifts` on sentence-initial `only`.
resource fixture:only_manner_adv : lexicon:LexicalEntry {
    lexicon:form     = "only";
    lexicon:cat      = type_expr( lexicon:cat_fin_forall(fun (f : lexicon:Fin) => lexicon:cat_num_forall(fun (n : lexicon:Num) => lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, f), lexicon:cat_np(lexicon:Entity, n)), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, f), lexicon:cat_np(lexicon:Entity, n))))) );
    lexicon:sem      = fixture:only_adv_sem;
    lexicon:sem_type = type_expr( (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop) );
    lexicon:sense    = "wn:only.r.00004722";
}

resource fixture:assay_n : lexicon:LexicalEntry {
    lexicon:form     = "assay";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Assay, lexicon:sg) );
    lexicon:sem      = fixture:Assay;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_assay";
}
resource fixture:pattern_n : lexicon:LexicalEntry {
    lexicon:form     = "pattern";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Pattern, lexicon:sg) );
    lexicon:sem      = fixture:Pattern;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_pattern";
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
resource fixture:study_n : lexicon:LexicalEntry {
    lexicon:form     = "study";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Study, lexicon:sg) );
    lexicon:sem      = fixture:Study;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "fixture_study";
}
resource fixture:safe_adj : lexicon:LexicalEntry {
    lexicon:form     = "safe";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:safe;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_safe";
}
resource fixture:essential_adj : lexicon:LexicalEntry {
    lexicon:form     = "essential";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:essential;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_essential";
}
resource fixture:involves_v : lexicon:LexicalEntry {
    lexicon:form     = "involves";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = fixture:involves;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fixture_involves";
}
"#;

fn parser() -> Parser {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("only-focus", Some(Arc::clone(ctx.head())));
    for src in [LAYER, FIXTURE] {
        for r in esl::compile(src, ctx.head()).expect("source compiles") {
            b.add_resource(r).expect("add resource");
        }
    }
    let layer: Arc<Layer> = Arc::new(b.build(LayerStorage::in_memory()));
    Parser::build(layer)
}

fn sems(sentence: &str) -> Vec<String> {
    let (closed, _open) = parser().parse_open(sentence, &Identity);
    closed.iter().map(|it| pretty_term(it.sem())).collect()
}

#[test]
fn subject_indefinite_and_cardinal_focus_restricts_with_sole_and_suppresses_adv_initial() {
    let indef = sems("only an assay is essential");
    assert!(
        indef
            .iter()
            .any(|s| s.contains("And(sole(G#1), essential(G#1))")),
        "expected `only an assay` to restrict witness with `sole`; got: {indef:#?}"
    );
    assert!(
        !indef.iter().any(|s| s.contains("only_adv")),
        "sentence-initial `only` must not double-seed `only_adv` via `adv_initial_lifts`: {indef:#?}"
    );

    let card = sems("only one pattern is safe");
    assert!(
        card.iter()
            .any(|s| s.contains("has_count(Pattern") && s.contains("And(sole(G#0), safe(G#0))")),
        "expected `only one pattern` to restrict counted witness with `sole`; got: {card:#?}"
    );
    assert!(
        !card.iter().any(|s| s.contains("only_adv")),
        "sentence-initial `only` must not double-seed `only_adv`: {card:#?}"
    );

    let def = sems("only the gene is essential");
    assert!(
        def.iter()
            .any(|s| s.contains("And(sole(the(Gene)), essential(the(Gene)))")),
        "expected `only the gene` to restrict definite referent with `sole`; got: {def:#?}"
    );
    assert!(
        !def.iter().any(|s| s.contains("only_adv")),
        "sentence-initial `only` must not double-seed `only_adv`: {def:#?}"
    );
}

#[test]
fn object_focus_closes_with_sole() {
    let obj = sems("the study involves only four genes");
    assert!(
        obj.iter().any(|s| {
            s.contains("has_count(Gene") && s.contains("And(sole(G#0), involves(G#0, the(Study)))")
        }),
        "expected `only four genes` in object position to close with `sole`; got: {obj:#?}"
    );
}
