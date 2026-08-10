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

//! **D66 slice 1 — the two ends of the witness key must hash the same term.**
//!
//! A `WitnessKey` carries `prop_hash`, and two places compute it:
//!
//! - the **check** side, during type-checking, from an already-evaluated `Val`
//!   (`kernel/src/program/check_hooks.rs` — `readback_val` then `WitnessKey::from_exp`);
//! - the **emit** side, deciding whether a layer admits the witness, from the proposition as stored.
//!
//! Before slice 1 the emit side hashed the *stored JSON* directly. That agreed with the check side
//! only while nothing could make the written form differ from the interpreted one — which is exactly
//! what transparent definitions introduce (D66 §4): the author writes a folded name, the checker sees
//! the unfolded body. Slice 1 makes the emit side decode first.
//!
//! What this file pins is the property that has to hold, on the shape the DCG parser actually emits.
//! The kernel-side test (`layer::witness_index::tests::emit_and_check_sides_agree_on_the_hash`)
//! covers the simple shapes; it cannot construct **the definite description**
//! `Fst(the(Σx. …))` — every parsed sentence contains one — because `ontology:the` is not in a
//! core-only layer, and `Fst` of a bare `Sig` is ill-typed (a projection of a *type*, not of a pair).
//! Building a chain that carries the `ontology` axioms is the whole reason this test lives here.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::{Exp, Patt};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::program::eigentt_type_mirror::{decode_type, encode_type};
use eigenius_kernel::witness::hash_proposition_exp;

/// A chain carrying the vocabulary a parsed sentence is built from: `lexicon:Entity`, the
/// `ontology:` axioms (`the`, `kind_of`, `prep_of`, `compound_kind`), and `logic:And`.
fn chain_with_parse_vocabulary() -> Arc<Layer> {
    let mut core = LayerBuilder::new("core", None);
    for r in eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
        .unwrap()
    {
        core.add_resource(r).unwrap();
    }
    let core = Arc::new(core.build(LayerStorage::in_memory()));

    let mut refl = LayerBuilder::new("reflection", Some(core));
    for src in [
        include_str!("../../../ontologies/reflection/reflection-ontology.json"),
        include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
    ] {
        for r in eigon_json::parse_document(src).unwrap() {
            refl.add_resource(r).unwrap();
        }
    }
    let refl = Arc::new(refl.build(LayerStorage::in_memory()));

    let mut vocab = LayerBuilder::new("parse-vocabulary", Some(refl));
    for src in [
        include_str!("../../../ontologies/logic/logic.esl"),
        include_str!("../../../ontologies/lexicon/lexicon-ontology.esl"),
        include_str!("../../../ontologies/ontology/ontology.esl"),
    ] {
        for r in esl::compile(src).expect("ontology ESL compiles") {
            vocab.add_resource(r).unwrap();
        }
    }
    // The two classes a sentence's arguments resolve to, standing in for a WordNet synset and a
    // UMLS concept.
    for r in esl::compile(
        r#"
        namespace cls = "urn:eigenius:demo:cls";
        class cls:WRN { }
        class cls:exonuclease { }
        class cls:activity { }
        class cls:model { }
    "#,
    )
    .unwrap()
    {
        vocab.add_resource(r).unwrap();
    }
    Arc::new(vocab.build(LayerStorage::in_memory()))
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}
fn ax(s: &str) -> Exp {
    Exp::EigonAxiom(iri(s))
}
fn cls(s: &str) -> Exp {
    Exp::EigonClass(iri(s))
}
fn app2(f: Exp, a: Exp, b: Exp) -> Exp {
    Exp::App(Box::new(Exp::App(Box::new(f), Box::new(a))), Box::new(b))
}

/// The real shape, mirroring `demo/prose-to-formulas/rules.esl`:
///
/// ```text
/// prep_of( fst(the(Σx0 : activity. And(compound_kind(x0, exonuclease),
///                                      prep_of(x0, kind_of(WRN))))),
///          kind_of(model) )
/// ```
///
/// `ontology:prep_of` stands in for the verb axiom — same `Entity -> Entity -> Prop` arrow a
/// transitive verb gets (`crates/eigenius-wordnet/src/convert.rs:210`), without needing a lexicon.
fn definite_description_parse() -> Exp {
    let inner = Exp::Sig(
        Patt::Var("x0".into()),
        Box::new(cls("urn:eigenius:demo:cls:activity")),
        Box::new(app2(
            ax("urn:eigenius:logic:And"),
            app2(
                ax("urn:eigenius:ontology:compound_kind"),
                Exp::Var("x0".into()),
                cls("urn:eigenius:demo:cls:exonuclease"),
            ),
            app2(
                ax("urn:eigenius:ontology:prep_of"),
                Exp::Var("x0".into()),
                Exp::App(
                    Box::new(ax("urn:eigenius:ontology:kind_of")),
                    Box::new(cls("urn:eigenius:demo:cls:WRN")),
                ),
            ),
        )),
    );
    app2(
        ax("urn:eigenius:ontology:prep_of"),
        Exp::Fst(Box::new(Exp::App(
            Box::new(ax("urn:eigenius:ontology:the")),
            Box::new(inner),
        ))),
        Exp::App(
            Box::new(ax("urn:eigenius:ontology:kind_of")),
            Box::new(cls("urn:eigenius:demo:cls:model")),
        ),
    )
}

/// Emit side, as of slice 1: decode the stored proposition, hash the resulting `Exp`.
fn emit_side_hash(layer: &Layer, stored: &Value) -> [u8; 32] {
    let decoded = decode_type(stored, layer).expect("stored proposition decodes");
    hash_proposition_exp(&decoded).expect("decoded proposition hashes")
}

/// Check side, as it already behaves: the proposition arrives evaluated, and is read back before
/// hashing.
fn check_side_hash(layer: &Layer, stored: &Value) -> [u8; 32] {
    let decoded = decode_type(stored, layer).expect("stored proposition decodes");
    let value = eval(&decoded, &Rho::Nil).expect("decoded proposition evaluates");
    hash_proposition_exp(&readback_val(0, &value)).expect("readback hashes")
}

/// The load-bearing test: on the shape every parsed sentence has, the two ends agree.
///
/// They differ by `eval` + `readback`. Readback freshens binder names, which
/// `alpha_canonicalize_proposition_json` absorbs (D66 D4). `eval` has nothing to do here: parses are
/// β-normal, and under D9 a definition's body is stored already normalized so decode yields a normal
/// term. If either of those stops holding, this fails.
#[test]
fn emit_and_check_agree_on_the_definite_description() {
    let layer = chain_with_parse_vocabulary();
    let prop = definite_description_parse();
    let stored = encode_type(&prop).expect("the parse shape encodes");

    assert_eq!(
        emit_side_hash(&layer, &stored),
        check_side_hash(&layer, &stored),
        "the emit and check sides must hash the definite-description shape identically"
    );
}

/// The negation the demo turns on — `⟨parse⟩ → False` — must agree too, and must **not** collide
/// with the un-negated form. Deleting one negation is the edit `demo/prose-to-formulas` shows the
/// kernel catching; it is caught precisely because the two hash differently.
#[test]
fn negation_agrees_and_does_not_collide() {
    let layer = chain_with_parse_vocabulary();
    let plain = definite_description_parse();
    let negated = Exp::Arrow(
        Box::new(plain.clone()),
        Box::new(Exp::EigonClass(iri("urn:eigenius:logic:False"))),
    );

    let plain_stored = encode_type(&plain).unwrap();
    let negated_stored = encode_type(&negated).unwrap();

    assert_eq!(
        emit_side_hash(&layer, &negated_stored),
        check_side_hash(&layer, &negated_stored),
        "the negated form must also agree across the two sides"
    );
    assert_ne!(
        emit_side_hash(&layer, &plain_stored),
        emit_side_hash(&layer, &negated_stored),
        "deleting a negation must change the hash — this is what makes the demo's edit detectable"
    );
}

/// Binder names must not affect the key. The DCG emits `x0`, `x1`, … while NbE readback freshens to
/// `G#0`, `G#1`, … — so without α-canonicalization the two sides could never meet (D66 D4).
#[test]
fn binder_renaming_does_not_change_the_key() {
    let layer = chain_with_parse_vocabulary();
    let prop = definite_description_parse();

    // The same term with its bound variable renamed.
    fn rename(e: &Exp, from: &str, to: &str) -> Exp {
        match e {
            Exp::Var(n) if n.as_str() == from => Exp::Var(to.to_string()),
            Exp::App(f, a) => {
                Exp::App(Box::new(rename(f, from, to)), Box::new(rename(a, from, to)))
            }
            Exp::Sig(Patt::Var(n), d, b) if n.as_str() == from => Exp::Sig(
                Patt::Var(to.to_string()),
                Box::new(rename(d, from, to)),
                Box::new(rename(b, from, to)),
            ),
            Exp::Fst(a) => Exp::Fst(Box::new(rename(a, from, to))),
            other => other.clone(),
        }
    }
    let renamed = rename(&prop, "x0", "G#0");
    assert_ne!(
        encode_type(&prop).unwrap(),
        encode_type(&renamed).unwrap(),
        "the two encodings must differ syntactically, or this proves nothing"
    );
    assert_eq!(
        emit_side_hash(&layer, &encode_type(&prop).unwrap()),
        emit_side_hash(&layer, &encode_type(&renamed).unwrap()),
        "alpha-variants must produce the same witness key"
    );
}

/// Guard against the agreement tests being vacuous.
///
/// They compare `decode → hash` against `decode → eval → readback → hash`. If `eval` + `readback`
/// were the identity on this term the comparison would prove nothing, so assert that the two paths
/// really do produce *different* `Exp`s and that the hash is what reconciles them.
#[test]
fn the_two_paths_are_actually_different() {
    let layer = chain_with_parse_vocabulary();
    let stored = encode_type(&definite_description_parse()).unwrap();

    let decoded = decode_type(&stored, &layer).unwrap();
    let round_tripped = readback_val(0, &eval(&decoded, &Rho::Nil).unwrap());

    assert_ne!(
        encode_type(&decoded).unwrap(),
        encode_type(&round_tripped).unwrap(),
        "eval + readback must change the term (it freshens binders); if it does not, the \
         agreement tests above are trivially true and prove nothing"
    );
}

// ── D66 slice 2: a transparent definition unfolds at decode ────────────────────────────────────

use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::ontology::well_known as wk;

/// Build a `eigentt:Definition` resource: `def F (g : Set) (a : Set) : Prop = prep_of(kind_of(g), kind_of(a))`.
///
/// The body is stored as a lambda chain, already normal (D9). `opaque` makes it rigid instead.
fn definition_resource(def_iri: &str, opaque: bool) -> Resource {
    let body = Exp::Lam(
        Patt::Var("g".into()),
        Box::new(Exp::Lam(
            Patt::Var("a".into()),
            Box::new(app2(
                ax("urn:eigenius:ontology:prep_of"),
                Exp::App(
                    Box::new(ax("urn:eigenius:ontology:kind_of")),
                    Box::new(Exp::Var("g".into())),
                ),
                Exp::App(
                    Box::new(ax("urn:eigenius:ontology:kind_of")),
                    Box::new(Exp::Var("a".into())),
                ),
            )),
        )),
    );
    // `Exp::Lam` carries no domain, so the encoder needs the annotations supplied separately.
    let encoded_body = eigenius_kernel::program::eigentt_type_mirror::encode_lam_chain(
        &[
            (Patt::Var("g".into()), Exp::Sort(1)),
            (Patt::Var("a".into()), Exp::Sort(1)),
        ],
        match &body {
            Exp::Lam(_, inner) => match inner.as_ref() {
                Exp::Lam(_, b) => b,
                other => other,
            },
            other => other,
        },
    )
    .expect("lambda chain encodes");

    let mut r = Resource::new(iri(def_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:eigentt:Definition",
        ))]),
    );
    r.set(
        iri("urn:eigenius:eigentt:definition_type"),
        encode_type(&Exp::Pi(
            Patt::Unit,
            Box::new(Exp::Sort(1)),
            Box::new(Exp::Pi(
                Patt::Unit,
                Box::new(Exp::Sort(1)),
                Box::new(Exp::Sort(0)),
            )),
        ))
        .unwrap(),
    );
    r.set(iri("urn:eigenius:eigentt:definition_body"), encoded_body);
    if opaque {
        r.set(
            iri("urn:eigenius:eigentt:definition_opaque"),
            Value::Boolean(true),
        );
    }
    r
}

fn chain_with_definition(def_iri: &str, opaque: bool) -> Arc<Layer> {
    let base = chain_with_parse_vocabulary();
    let mut b = LayerBuilder::new("definitions", Some(base));
    b.add_resource(definition_resource(def_iri, opaque))
        .unwrap();
    Arc::new(b.build(LayerStorage::in_memory()))
}

const DEF: &str = "urn:eigenius:demo:def:HasActivity";

/// The load-bearing slice-2 test: a use of a transparent definition decodes to its unfolded body,
/// with the arguments substituted and **no beta-redex** left behind.
#[test]
fn a_transparent_definition_unfolds_at_decode() {
    let layer = chain_with_definition(DEF, false);

    // `F(WRN, exonuclease)` as it would be stored: an App spine over the definition's IRI.
    let use_site = app2(
        Exp::EigonAxiom(iri(DEF)), // encodes as ConstRef; decode discriminates by class
        cls("urn:eigenius:demo:cls:WRN"),
        cls("urn:eigenius:demo:cls:exonuclease"),
    );
    let stored = encode_type(&use_site).unwrap();
    let decoded = decode_type(&stored, &layer).expect("the use decodes");

    // What the body means once instantiated.
    let expected = app2(
        ax("urn:eigenius:ontology:prep_of"),
        Exp::App(
            Box::new(ax("urn:eigenius:ontology:kind_of")),
            Box::new(cls("urn:eigenius:demo:cls:WRN")),
        ),
        Exp::App(
            Box::new(ax("urn:eigenius:ontology:kind_of")),
            Box::new(cls("urn:eigenius:demo:cls:exonuclease")),
        ),
    );
    assert_eq!(decoded, expected, "the definition must unfold to its body");

    // The point of peel-and-substitute: no redex is ever formed.
    fn has_redex(e: &Exp) -> bool {
        match e {
            Exp::App(f, a) => matches!(f.as_ref(), Exp::Lam(..)) || has_redex(f) || has_redex(a),
            Exp::Fst(x) | Exp::Snd(x) => has_redex(x),
            Exp::Sig(_, d, b) | Exp::Pi(_, d, b) => has_redex(d) || has_redex(b),
            Exp::Lam(_, b) => has_redex(b),
            _ => false,
        }
    }
    assert!(
        !has_redex(&decoded),
        "peel-and-substitute must not leave a beta-redex: {decoded:?}"
    );
}

/// An opaque definition does NOT unfold — it stays rigid, like an axiom (#95 / D9 carve-out).
#[test]
fn an_opaque_definition_stays_folded() {
    let layer = chain_with_definition(DEF, true);
    let use_site = app2(
        Exp::EigonAxiom(iri(DEF)),
        cls("urn:eigenius:demo:cls:WRN"),
        cls("urn:eigenius:demo:cls:exonuclease"),
    );
    let stored = encode_type(&use_site).unwrap();
    let decoded = decode_type(&stored, &layer).expect("the use decodes");
    assert_eq!(
        decoded, use_site,
        "an opaque definition must decode to itself, unfolded by nothing"
    );
}

/// Folded and unfolded forms hash **identically** — the property the whole design turns on. An
/// author writes the definition; the checker sees the parse; the witness key must not care.
#[test]
fn folded_and_unfolded_uses_hash_the_same() {
    let layer = chain_with_definition(DEF, false);
    let folded = encode_type(&app2(
        Exp::EigonAxiom(iri(DEF)),
        cls("urn:eigenius:demo:cls:WRN"),
        cls("urn:eigenius:demo:cls:exonuclease"),
    ))
    .unwrap();
    let unfolded = encode_type(&app2(
        ax("urn:eigenius:ontology:prep_of"),
        Exp::App(
            Box::new(ax("urn:eigenius:ontology:kind_of")),
            Box::new(cls("urn:eigenius:demo:cls:WRN")),
        ),
        Exp::App(
            Box::new(ax("urn:eigenius:ontology:kind_of")),
            Box::new(cls("urn:eigenius:demo:cls:exonuclease")),
        ),
    ))
    .unwrap();

    assert_ne!(folded, unfolded, "the stored forms differ, as they must");
    assert_eq!(
        emit_side_hash(&layer, &folded),
        emit_side_hash(&layer, &unfolded),
        "a definition's identity is the normal form of its RHS (D9), so the two must agree"
    );
}
