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

//! Adverbs import at Luo & Shi's `ADV` type (D62 §8.7.5, reopened by eigenius#270).
//!
//! `data.adv` was deferred because there was no type for a predicate modifier. #270 adopted Luo &
//! Shi 2026 and `ontology:adv_*` instantiates `ADV = (e -> t) -> (e -> t)` at a PP's object; a bare
//! adverb is the same thing without the object.
//!
//! What the emitted layer has to satisfy: it compiles over the real bootstrap, the axiom carries the
//! `ADV` type, the entry's sem is a term that KERNEL-GATES at that type, and the modifier conjoins
//! rather than vanishing — the identity sem the parser's derivational rule seeds is what this
//! replaces.

use std::sync::Arc;

use eigenius_kernel::dcg::lexicon::gate_entry;
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::validation::Validator;
use eigenius_wordnet::convert::{render_document, MassNouns, SenseRanks};
use eigenius_wordnet::governance::Governance;
use eigenius_wordnet::wndb::parse_data_line;

/// One adverb synset, rendered and stood up over the bootstrap.
fn adverb_layer() -> (String, Arc<Layer>) {
    let syn = parse_data_line("00064889 02 r 01 deliberately 0 001 \\ 00748310 a 0000 | with intention; in an intentional manner").expect("parse data.adv line");
    let (doc, rep) = render_document(
        &[syn],
        &SenseRanks::new(),
        &MassNouns::new(),
        &Governance::default(),
    );
    assert_eq!(rep.adv_axioms, 1, "one adverb synset → one ADV axiom");
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let resources = esl::compile(&doc, ctx.head())
        .unwrap_or_else(|e| panic!("adverb ESL compiles: {e:?}\n{doc}"));
    let mut b = LayerBuilder::new("wn-adv", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    (doc, Arc::new(b.build(LayerStorage::in_memory())))
}

#[test]
fn an_adverb_synset_compiles_and_validates() {
    let (doc, layer) = adverb_layer();
    let errors: Vec<String> = Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "the adverb layer validates: {errors:#?}\n{doc}"
    );
}

/// The axiom is at `ADV`, and the entry's sem is the CONJOINING term — not the identity the
/// derivational rule seeds. Modifier drop then follows from the entry, which is Luo & Shi's point.
#[test]
fn the_adverb_is_typed_adv_and_conjoins() {
    let (doc, _layer) = adverb_layer();
    assert!(
        doc.contains(
            "axiom wn:deliberately_r_00064889 : (lexicon:Entity -> Prop) -> lexicon:Entity -> Prop"
        ) || doc.contains("(lexicon:Entity -> Prop) -> lexicon:Entity -> Prop"),
        "the axiom carries Luo & Shi's ADV type:\n{doc}"
    );
    assert!(
        doc.contains("logic:And(V(s), wn:"),
        "the entry conjoins the modifier instead of dropping it:\n{doc}"
    );
    assert!(
        !doc.contains("fun (x : lexicon:Entity) => x"),
        "no identity sem survives in an imported adverb:\n{doc}"
    );
}

/// Both manner positions ship: the forward pre-modifier `(S\\NP)/(S\\NP)` and the backward
/// post-modifier `(S\\NP)\\(S\\NP)`. «partially exposed» needs the first, «confirms independently»
/// the second.
#[test]
fn both_manner_positions_ship() {
    let (doc, _) = adverb_layer();
    let vp = "lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any))";
    assert!(
        doc.contains(&format!("lexicon:fwd(lexicon:m_all, {vp}, {vp})")),
        "forward pre-modifier:\n{doc}"
    );
    assert!(
        doc.contains(&format!("lexicon:bwd(lexicon:m_all, {vp}, {vp})")),
        "backward post-modifier:\n{doc}"
    );
}

/// Every emitted entry must pass the FELICITY GATE — `⟦cat⟧ ≡ sem_type` and the sem inhabits
/// `⟦cat⟧`. This is the gate the importer routes through at commit, so an entry that fails it would
/// be a fail-closed finding rather than a silent drop; asserting it here keeps that off the reseed.
#[test]
fn every_adverb_entry_passes_the_felicity_gate() {
    let (doc, layer) = adverb_layer();
    let mut gated = 0;
    for (iri, r) in layer.iter_resources() {
        if !r.is_instance_of(
            &eigenius_kernel::ontology::Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap(),
        ) {
            continue;
        }
        let cat = gate_entry(&layer, &r)
            .unwrap_or_else(|e| panic!("{iri} must pass the felicity gate: {e}\n{doc}"));
        // ⟦cat⟧ must be `(Entity -> Prop) -> (Entity -> Prop)` — Luo & Shi's ADV. Checked on the
        // TERM, not its printed form: `pretty_term` renders `Arrow(a, b)` as `a → b` with no
        // parentheses on the left, so this type and `Entity -> (Prop -> (Entity -> Prop))` print
        // identically.
        let entity_to_prop = |e: &Exp| {
            matches!(e, Exp::Arrow(a, b)
                if matches!(**a, Exp::EigonClass(ref i) if i.as_str() == "urn:eigenius:lexicon:Entity")
                    && matches!(**b, Exp::Sort(ref l) if l.is_nat(0)))
        };
        match &cat {
            Exp::Arrow(arg, res) if entity_to_prop(arg) && entity_to_prop(res) => {}
            other => panic!(
                "{iri}: ⟦cat⟧ must be ADV = (Entity -> Prop) -> (Entity -> Prop), got {:?}",
                other
            ),
        }
        gated += 1;
    }
    assert_eq!(gated, 2, "one lemma x two manner positions");
}
