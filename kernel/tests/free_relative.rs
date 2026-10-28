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
//! **Free relatives** — `what` as a headless relative NP. Snapshot-free: the entries under
//! measurement come from `experiments/lexicon/free-relative.esl` (the very file layered onto a
//! snapshot for the DB measurement), and this fixture adds only the content words — two transitive
//! verbs and two NPs. The committed closed-class layer supplies the INTERROGATIVE `what`
//! (`what_subj` / `what_obj`), which is what makes the last test here meaningful.
//!
//! The point the tests pin is that a free relative is a DEFINITE DESCRIPTION, not a question and
//! not an existential: `what affects cells` must come out `the(Σx:Entity. affects(cells, x)).1` —
//! the ι operator over the Σ the body builds, projected to an entity. The interrogative `what`
//! (`what_subj`, `cat_q(Entity)`) shares the surface and the body category, so a reading that
//! leaked from it would be visible here as a `cat_q` parse of the whole sentence.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

/// The entries under measurement, compiled from the SAME file `add-layer-to-snapshot.sh` layers
/// onto a snapshot — one source of truth, so this test cannot pass against a version of the
/// entries that the measured layer does not contain.
const LAYER: &str = include_str!("../../experiments/lexicon/free-relative.esl");

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace core    = "urn:eigenius:core";

// ── Content words ────────────────────────────────────────────────────────────

axiom lexicon:affects  : lexicon:Entity -> lexicon:Entity -> Prop
axiom lexicon:reported : lexicon:Entity -> lexicon:Entity -> Prop
axiom lexicon:cells    : lexicon:Entity
axiom lexicon:study    : lexicon:Entity

resource lexicon:fr_cells : lexicon:LexicalEntry {
    lexicon:form = "cells"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:pl) );
    lexicon:sem = lexicon:cells; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "fr_cells";
}
resource lexicon:fr_study : lexicon:LexicalEntry {
    lexicon:form = "study"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem = lexicon:study; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "fr_study";
}
// 3sg finite transitive — SINGULAR subject slot, `num_any` object (agreement is subject-only).
resource lexicon:fr_affects : lexicon:LexicalEntry {
    lexicon:form     = "affects";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:affects;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fr_affects";
}
resource lexicon:fr_reported : lexicon:LexicalEntry {
    lexicon:form     = "reported";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:sg)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:reported;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "fr_reported";
}
"#;

fn parser() -> Parser {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("free-rel", Some(Arc::clone(ctx.head())));
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
    closed.iter().map(|p| pretty_term(p.sem())).collect()
}

/// SUBJECT gap, the battery's own case: «The study reported what affects cells.» The free relative
/// is the verb's object, and it denotes the maximal affector — `the(Σx. affects(cells, x)).1`,
/// NOT an existential and NOT the interrogative's answer-property.
#[test]
fn a_subject_gap_free_relative_is_a_definite_description() {
    let got = sems("study reported what affects cells");
    assert!(
        got.iter().any(|s| {
            s.contains("the(Σ") && s.contains("affects(cells") && s.contains("reported")
        }),
        "subject-gap free relative must give reported(the(Σx. affects(cells, x)).1, study); got {got:?}"
    );
}

/// The ι is PROJECTED: the free relative fills an `Entity` argument slot, so the `the(Σ)` pair must
/// be `Fst`-projected (`.1`) exactly as a definite over a refined noun is. Without the projection
/// the sem would be a pair where the verb wants an entity.
#[test]
fn the_free_relatives_iota_is_projected_to_an_entity() {
    let got = sems("study reported what affects cells");
    assert!(
        got.iter().any(|s| s.contains(").1")),
        "the free relative's ι must be Fst-projected to an Entity; got {got:?}"
    );
}

/// A free relative is a CLAUSE-sized NP, so the whole sentence must still be an `S` that the
/// felicity gate closes — a `cat_q` reading leaking from the interrogative `what` would not be a
/// closed declarative parse.
#[test]
fn the_sentence_containing_a_free_relative_closes_as_a_declarative() {
    assert!(
        !sems("study reported what affects cells").is_empty(),
        "a free relative in object position must yield a CLOSED declarative parse"
    );
}

/// The bare string stays a QUESTION. `what affects cells` on its own closes only as the
/// interrogative `what_subj`'s answer-property `λx. affects(cells, x)` (`cat_q(Entity)`) — the
/// free relative is an NP and contributes no standalone reading, so the new entries did not turn a
/// wh-question into a statement. The two `what` senses coexist and the SPAN selects between them:
/// the same body yields a question at the top and a definite description under a verb.
#[test]
fn the_bare_string_still_closes_only_as_a_question() {
    let got = sems("what affects cells");
    assert_eq!(
        got.len(),
        1,
        "the bare wh-string must keep exactly its one interrogative reading; got {got:?}"
    );
    assert!(
        got[0].starts_with('λ') && !got[0].contains("the(Σ"),
        "that reading must be the answer-property, not a definite description; got {got:?}"
    );
}

/// OBJECT gap: «what the study reported» — the body is a declarative missing its OBJECT
/// (`S[dcl]/NP`), which is the same category `relativize` already takes for a headed object
/// relative. The entry differs from the subject-gap one only in the slash, and shares its sem, so
/// this pins that `⟦S/NP⟧ = ⟦S\NP⟧ = Entity → Prop` really does let one term serve both.
#[test]
fn an_object_gap_free_relative_builds_the_same_definite() {
    let got = sems("study reported what study affects");
    assert!(
        got.iter()
            .any(|s| s.contains("the(Σ") && s.contains("affects(") && s.contains(").1")),
        "object-gap free relative must give reported(the(Σx. affects(x, study)).1, study); got {got:?}"
    );
}
