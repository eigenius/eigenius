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
//! **Superlative constructions** end to end, through the real chart
//! (`docs/notes/d63-comparative-phrasal.md` §7a slice 1, §7b): attributive (`the most <adj> N`),
//! POST-NOMINAL (`the N most <adj> to X`) and PARTITIVE (`the most <adj> of the <group>`). The unit
//! tests in `combinators.rs` pin each rule's sem in isolation; these pin that the leaf functor, the
//! rules and the definite article actually assemble.
//!
//! Snapshot-free: the fixture supplies a gradable adjective's `cat_measure` scale, a count noun and
//! a transitive verb. The committed closed-class layer supplies `the` (`the_plain_np_pl`,
//! `the_plain_np`) and the QUANTIFIER `most` — the homograph this construction has to coexist with.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";
namespace fixture = "urn:eigenius:fixture";

// A common noun's type is a CLASS under `Entity` — what a WordNet synset is in the real lexicon.
class fixture:Trap : lexicon:Entity { }
class fixture:Function : lexicon:Entity { }
axiom lexicon:disease_e : lexicon:Entity

axiom lexicon:deg_dangerous : lexicon:Entity -> core:float
// A RELATIONAL gradable adjective: it is a scale only once its `to`-argument is consumed. This is
// the `cat_measure/cat_pp_arg(prep)` reading a WordNet adjective gets at seed time, and the shape
// the two-token superlative seed could not reach.
axiom lexicon:deg_relevant  : lexicon:Entity -> lexicon:Entity -> core:float
axiom lexicon:reported      : lexicon:Entity -> lexicon:Entity -> Prop
axiom lexicon:study         : lexicon:Entity

// The adjective's SCALE — the `cat_measure` reading a gradable adjective gets at seed time, and the
// one `most <adj>` consumes.
resource lexicon:ps_dangerous : lexicon:LexicalEntry {
    lexicon:form     = "dangerous";
    lexicon:cat      = type_expr( lexicon:cat_measure );
    lexicon:sem      = lexicon:deg_dangerous;
    lexicon:sem_type = type_expr( lexicon:Entity -> core:float );
    lexicon:sense    = "ps_dangerous";
}
resource lexicon:ps_relevant : lexicon:LexicalEntry {
    lexicon:form     = "relevant";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:cat_measure, lexicon:cat_pp_arg(lexicon:prep_to)) );
    lexicon:sem      = lexicon:deg_relevant;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> core:float );
    lexicon:sense    = "ps_relevant";
}
resource lexicon:ps_disease : lexicon:LexicalEntry {
    lexicon:form     = "disease";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem      = lexicon:disease_e;
    lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense    = "ps_disease";
}
resource lexicon:ps_function : lexicon:LexicalEntry {
    lexicon:form     = "function";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Function, lexicon:sg) );
    lexicon:sem      = fixture:Function;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "ps_function";
}
resource lexicon:ps_traps : lexicon:LexicalEntry {
    lexicon:form     = "traps";
    lexicon:cat      = type_expr( lexicon:cat_n(fixture:Trap, lexicon:pl) );
    lexicon:sem      = fixture:Trap;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "ps_traps";
}
resource lexicon:ps_study : lexicon:LexicalEntry {
    lexicon:form     = "study";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem      = lexicon:study;
    lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense    = "ps_study";
}
resource lexicon:ps_reported : lexicon:LexicalEntry {
    lexicon:form     = "reported";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:reported;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "ps_reported";
}
"#;

fn sems(sentence: &str) -> Vec<String> {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("partitive-sup", Some(Arc::clone(ctx.head())));
    for r in esl::compile(FIXTURE, ctx.head()).expect("fixture compiles") {
        b.add_resource(r).expect("add resource");
    }
    let layer: Arc<Layer> = Arc::new(b.build(LayerStorage::in_memory()));
    let (closed, _open) = Parser::build(layer).parse_open(sentence, &Identity);
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

/// The construction, end to end: the fused head takes its maximality over the GROUP the `of`-phrase
/// names, through `has_proportion(g, …, 1)` — no `argmax_over`, no `member_of`, no new axiom.
#[test]
fn a_partitive_superlative_maximizes_over_the_group() {
    let got = sems("study reported the most dangerous of the traps");
    assert!(
        got.iter()
            .any(|s| s.contains("has_proportion") && s.contains("deg_dangerous")),
        "the partitive superlative must reach has_proportion over the adjective's scale; got {got:?}"
    );
}

/// The group is the `of`-phrase's DEFINITE, and the maximality quantifies over it — not over the
/// type, which would read the sentence as maximal among traps in general.
#[test]
fn the_comparison_class_is_the_group_entity_not_the_type() {
    let got = sems("study reported the most dangerous of the traps");
    let hit = got
        .iter()
        .find(|s| s.contains("has_proportion"))
        .unwrap_or_else(|| panic!("no partitive reading; got {got:?}"));
    assert!(
        hit.contains("has_proportion(the("),
        "has_proportion's group must be the definite `the(Trap)`; got {hit}"
    );
}

/// The ATTRIBUTIVE superlative is untouched: `most <adj>` + a head noun still quantifies over the
/// noun's TYPE (slice 1), and the new three-token seed did not shadow the two-token one.
#[test]
fn the_attributive_superlative_still_quantifies_over_the_head_nouns_type() {
    let got = sems("study reported the most dangerous traps");
    assert!(
        got.iter()
            .any(|s| s.contains("deg_dangerous") && !s.contains("has_proportion")),
        "`most dangerous traps` must keep slice 1's type-level reading; got {got:?}"
    );
}

/// `most` is also the QUANTIFIER determiner, and `of` the ordinary preposition. The three-token span
/// is deliberately unprotected, so neither homograph is shadowed — "most of the traps" keeps the
/// partitive-quantifier reading it had, with no superlative in sight.
#[test]
fn the_quantifier_most_is_not_shadowed() {
    let got = sems("study reported most of the traps");
    assert!(
        !got.iter().any(|s| s.contains("deg_dangerous")),
        "the quantifier reading must not acquire a scale; got {got:?}"
    );
}

/// **POST-NOMINAL superlative over a PHRASAL scale** — "the function most relevant to disease".
/// `relevant` is a `cat_measure/cat_pp_arg(to)`, so its scale only exists once the `to`-phrase is
/// consumed, three tokens wide. Slice 1's two-token seed read the scale out of the adjective's LEAF
/// cell and so could not see it; `most` as a `cat_sup_mod/cat_measure` functor composes by ordinary
/// forward application at any span.
#[test]
fn a_superlative_composes_with_a_phrasal_scale_and_attaches_post_nominally() {
    let got = sems("study reported the function most relevant to disease");
    assert!(
        got.iter()
            .any(|s| s.contains("deg_relevant") && s.contains("ge(")),
        "the post-nominal superlative must build a maximality over the relational scale; got {got:?}"
    );
}

/// The comparison class is the HEAD NOUN's type, exactly as in the pre-nominal rule — the only
/// difference between them is which operand the noun is.
#[test]
fn the_post_nominal_superlatives_class_is_the_head_noun() {
    let got = sems("study reported the function most relevant to disease");
    let hit = got
        .iter()
        .find(|s| s.contains("deg_relevant"))
        .unwrap_or_else(|| panic!("no post-nominal superlative reading; got {got:?}"));
    assert!(
        hit.contains("Function"),
        "the ∀ must range over the head noun's type; got {hit}"
    );
}

/// The bare post-nominal ADJECTIVE stays out. This is what the `reduced_relative` `adj` widening
/// licensed and this rule does not: the trigger is `cat_sup_mod`, which only `most` produces.
#[test]
fn a_bare_post_nominal_adjective_is_still_refused() {
    assert!(
        sems("study reported the traps dangerous").is_empty(),
        "a bare post-nominal adjective must not attach"
    );
}

/// The post-nominal slot takes a COMPLEMENT-bearing scale only. "most want" is `most` over a bare
/// one-token `cat_measure` (WordNet reads `want` as the adjective *needy*), so it reaches
/// `cat_sup_mod` and NOT `cat_sup_mod_post` — the derivation that gave a page sentence with an
/// ADVERBIAL `most` 40 destitution-maximizing readings does not exist, rather than being filtered.
#[test]
fn a_bare_superlative_cannot_attach_post_nominally() {
    let got = sems("study reported the traps most dangerous");
    assert!(
        got.is_empty(),
        "a complement-less superlative must not attach post-nominally; got {got:?}"
    );
}

/// …while the relational one still does, and keeps the adjective's own governed preposition: the
/// `P` in `(cat_sup_mod_post/cat_pp_arg(P)) / (cat_measure/cat_pp_arg(P))` is a feature VARIABLE,
/// not `prep_any`.
#[test]
fn the_relational_superlative_threads_the_governed_preposition() {
    assert!(
        !sems("study reported the function most relevant to disease").is_empty(),
        "`most relevant to X` must still compose"
    );
}
