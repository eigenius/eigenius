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

//! End-to-end witness of the lookup bridge (D62 §8.8.1) driven by WordNet's
//! Morphy: an **inflected** prose sentence → the forest of typed parses. The
//! lexicon stores the verb at its **base** form (`affect`); the input inflects it
//! (`affects`); [`MorphyLemmatizer`] bridges the two so the bridge can look the
//! entry up and compose to a kernel-checked `S`. A control with the trivial
//! `Identity` lemmatizer (which does *not* reduce) yields no parse, isolating
//! Morphy's contribution.

use std::sync::Arc;

use eigenius_kernel::dcg::{Identity, Parser};
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::{bootstrap, esl};
use eigenius_wordnet::lemmatizer::MorphyLemmatizer;
use eigenius_wordnet::morphy::{ExcLists, LemmaSet};
use eigenius_wordnet::wndb::Pos;

// A minimal domain over the lexicon schema (now bootstrapped, D62/D63). The verb entry's `form` is the BASE
// lemma "affect" (as the WordNet import emits), typed at the `Entity` supertype so
// `Gene`/`CellLine` arguments flow in by subsumption; the proper nouns are NP
// individuals.
const DOMAIN: &str = r#"
namespace core      = "urn:eigenius:core";
namespace epistemic = "urn:eigenius:reflection:epistemic";
namespace lexicon   = "urn:eigenius:lexicon";

class lexicon:Entity { description = "top of the demo entity hierarchy"; }
class lexicon:Gene : lexicon:Entity { description = "a gene"; }
class lexicon:CellLine : lexicon:Entity { description = "a cell line"; }

axiom lexicon:affect : lexicon:Entity -> lexicon:Entity -> Prop

resource lexicon:brca1 : lexicon:Gene { core:description = "the BRCA1 gene"; }
resource lexicon:hela : lexicon:CellLine { core:description = "the HeLa cell line"; }

resource lexicon:e_affect : lexicon:LexicalEntry {
    lexicon:form     = "affect";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:affect;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "wn:affect.v.01";
    lexicon:grade    = epistemic:declared;
}
resource lexicon:e_brca1 : lexicon:LexicalEntry {
    lexicon:form     = "BRCA1";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:num_any) );
    lexicon:sem      = lexicon:brca1;
    lexicon:sem_type = type_expr( lexicon:Gene );
    lexicon:sense    = "urn:eigenius:lexicon:brca1";
    lexicon:grade    = epistemic:declared;
}
resource lexicon:e_hela : lexicon:LexicalEntry {
    lexicon:form     = "HeLa";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:CellLine, lexicon:num_any) );
    lexicon:sem      = lexicon:hela;
    lexicon:sem_type = type_expr( lexicon:CellLine );
    lexicon:sense    = "urn:eigenius:lexicon:hela";
    lexicon:grade    = epistemic:declared;
}
"#;

fn layer_over(name: &str, parent: Arc<Layer>, src: &str) -> Arc<Layer> {
    let resources = esl::compile_against_layer(src, &parent)
        .unwrap_or_else(|e| panic!("{name} must compile: {e:?}"));
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r)
            .unwrap_or_else(|e| panic!("{name} add_resource: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn morphy() -> MorphyLemmatizer {
    let mut lemmas = LemmaSet::new();
    for v in ["affect", "depend"] {
        lemmas.insert(v, Pos::Verb);
    }
    MorphyLemmatizer::new(ExcLists::parse("", "", "", ""), lemmas)
}

#[test]
fn morphy_bridge_parses_inflected_sentence() {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    // The lexicon schema is part of the bootstrap chain now (D62/D63); build the
    // demo domain directly over the bootstrapped head.
    let domain = layer_over("lexicon-domain", Arc::clone(ctx.head()), DOMAIN);
    let index = Parser::build(domain);

    // The verb is INFLECTED in the input ("affects"); the entry's form is the base
    // "affect". Morphy reduces affects→affect, the bridge looks the entry up, and
    // composes to exactly one S (Gene/CellLine ⊑ Entity by subsumption), which the
    // kernel confirms inhabits Prop.
    let forest = index.parse("HeLa affects BRCA1", &morphy());
    assert_eq!(
        forest.len(),
        1,
        "Morphy must bridge the inflected 'affects' to the base entry 'affect'; got {}",
        forest.len()
    );
}

#[test]
fn identity_lemmatizer_cannot_reach_the_base_entry() {
    // Control isolating Morphy's contribution: the trivial lemmatizer does not
    // reduce, so the inflected surface "affects" never matches the base form
    // "affect" — no parse. (With the base surface "affect" it would.)
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    // The lexicon schema is part of the bootstrap chain now (D62/D63); build the
    // demo domain directly over the bootstrapped head.
    let domain = layer_over("lexicon-domain", Arc::clone(ctx.head()), DOMAIN);
    let index = Parser::build(domain);

    assert!(
        index.parse("HeLa affects BRCA1", &Identity).is_empty(),
        "without morphology the inflected surface must not resolve to the base entry"
    );
}
