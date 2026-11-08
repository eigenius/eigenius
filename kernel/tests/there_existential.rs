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

//! **Existential `there`** (`ontologies/lexicon/closed-class.esl::there_expl`) —
//! snapshot-free integration test.
//!
//! Pins:
//! 1. **Singular existential (`There is a gene.`)**: closes as `∃x:Gene. Eq(Entity, x, x)`.
//! 2. **Plural existential (`There are genes.`)**: closes as `∃x:Gene. Eq(Entity, x, x)`.
//! 3. **Negative existential (`There is no gene.`)**: closes as `∀x:Gene. Eq(Entity, x, x) -> False`.
//! 4. **Number-agreement rejection (`*There is genes.`, `*There are a gene.`)**: refused.
//! 5. **Non-copular rejection (`*There affects a gene.`)**: refused.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

class fixture:Gene : lexicon:Entity { }

axiom fixture:affects : lexicon:Entity -> lexicon:Entity -> Prop

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
    let mut b = LayerBuilder::new("there-existential-fixture", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("source compiles") {
        b.add_resource(r).expect("add resource");
    }
    Parser::build(Arc::new(b.build(LayerStorage::in_memory())))
}

fn sems(sentence: &str) -> Vec<String> {
    let (closed, _open) = parser().parse_open(sentence, &Identity);
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

#[test]
fn singular_there_is_a_gene_parses() {
    let got = sems("There is a gene.");
    assert!(
        !got.is_empty(),
        "expected a closed parse for 'There is a gene.'; got none"
    );
    assert!(
        got.iter().any(|s| s.contains("Gene") && s.contains("=")),
        "expected existential over Gene in 'There is a gene.'; got: {got:#?}"
    );
}

#[test]
fn plural_there_are_genes_parses() {
    let got = sems("There are genes.");
    assert!(
        !got.is_empty(),
        "expected a closed parse for 'There are genes.'; got none"
    );
    assert!(
        got.iter().any(|s| s.contains("Gene") && s.contains("=")),
        "expected existential over Gene in 'There are genes.'; got: {got:#?}"
    );
}

#[test]
fn negative_there_is_no_gene_parses() {
    let got = sems("There is no gene.");
    assert!(
        !got.is_empty(),
        "expected a closed parse for 'There is no gene.'; got none"
    );
    assert!(
        got.iter()
            .any(|s| s.contains("Gene") && s.contains("False")),
        "expected negative existential in 'There is no gene.'; got: {got:#?}"
    );
}

#[test]
fn there_existential_enforces_number_and_copula_constraints() {
    assert!(
        sems("There is genes.").is_empty(),
        "singular 'is' must refuse plural 'genes' in 'There is genes.'"
    );
    assert!(
        sems("There are a gene.").is_empty(),
        "plural 'are' must refuse singular 'a gene' in 'There are a gene.'"
    );
    assert!(
        sems("There affects a gene.").is_empty(),
        "expletive 'there' must refuse non-copular transitive verb in 'There affects a gene.'"
    );
}
