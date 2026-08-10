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

/// The real shape the DCG parser produces (what the demo's generated shape rules used to abstract):
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

// ── D66 slice 2, Rule 24: commit-time validation of a definition ───────────────────────────────

use eigenius_kernel::validation::{ValidationRule, Validator};

/// Validate `r` against a chain that already carries the parse vocabulary, returning the Rule 24
/// messages (if any).
fn definition_errors(r: Resource) -> Vec<String> {
    let base = chain_with_parse_vocabulary();
    Validator::new(Arc::clone(&base))
        .validate_resource(&r)
        .into_iter()
        .filter(|e| e.rule == ValidationRule::DefinitionMalformed)
        .map(|e| e.message)
        .collect()
}

/// A well-formed definition passes.
#[test]
fn rule24_accepts_a_well_formed_definition() {
    let errs = definition_errors(definition_resource(DEF, false));
    assert!(errs.is_empty(), "expected no Rule 24 errors, got {errs:?}");
}

/// Recursion is refused: decode substitutes the body at the use site, so a self-reference would
/// expand forever. There is no fuel and no termination argument (#66).
#[test]
fn rule24_rejects_a_recursive_definition() {
    let mut r = definition_resource(DEF, false);
    // A body that names its own IRI.
    let self_ref = eigenius_kernel::program::eigentt_type_mirror::encode_lam_chain(
        &[
            (Patt::Var("g".into()), Exp::Sort(1)),
            (Patt::Var("a".into()), Exp::Sort(1)),
        ],
        &app2(
            Exp::EigonAxiom(iri(DEF)),
            Exp::Var("g".into()),
            Exp::Var("a".into()),
        ),
    )
    .unwrap();
    r.set(iri("urn:eigenius:eigentt:definition_body"), self_ref);

    let errs = definition_errors(r);
    assert!(
        errs.iter().any(|m| m.contains("references its own IRI")),
        "a recursive body must be refused; got {errs:?}"
    );
}

/// A body carrying a beta-redex is refused: D9 makes a definition's identity the NORMAL FORM of its
/// right-hand side, so a redex-bearing body would hash differently on the two ends of the key.
#[test]
fn rule24_rejects_a_body_that_is_not_in_normal_form() {
    let mut r = definition_resource(DEF, false);
    // `(λz. kind_of(z)) WRN` — a redex, left unreduced.
    let redex = eigenius_kernel::program::eigentt_type_mirror::encode_lam_chain(
        &[
            (Patt::Var("g".into()), Exp::Sort(1)),
            (Patt::Var("a".into()), Exp::Sort(1)),
        ],
        &app2(
            ax("urn:eigenius:ontology:prep_of"),
            Exp::App(
                Box::new(
                    eigenius_kernel::program::eigentt_type_mirror::decode_type(
                        &eigenius_kernel::program::eigentt_type_mirror::encode_lam_chain(
                            &[(Patt::Var("z".into()), Exp::Sort(1))],
                            &Exp::App(
                                Box::new(ax("urn:eigenius:ontology:kind_of")),
                                Box::new(Exp::Var("z".into())),
                            ),
                        )
                        .unwrap(),
                        &chain_with_parse_vocabulary(),
                    )
                    .unwrap(),
                ),
                Box::new(cls("urn:eigenius:demo:cls:WRN")),
            ),
            Exp::App(
                Box::new(ax("urn:eigenius:ontology:kind_of")),
                Box::new(Exp::Var("a".into())),
            ),
        ),
    );
    // `encode_lam_chain` refuses a bare inner `Lam`, so if the redex cannot even be encoded the
    // invariant is enforced one layer earlier — which is also acceptable. Only assert when it can.
    if let Ok(encoded) = redex {
        r.set(iri("urn:eigenius:eigentt:definition_body"), encoded);
        let errs = definition_errors(r);
        assert!(
            errs.iter().any(|m| m.contains("not in normal form")),
            "a redex-bearing body must be refused; got {errs:?}"
        );
    }
}

/// The check that gives `definition_opaque` its meaning: the body must inhabit the declared type.
/// An axiom is asserted; a definition's body is verified and only then sealed.
#[test]
fn rule24_rejects_a_body_that_does_not_inhabit_its_declared_type() {
    let mut r = definition_resource(DEF, false);
    // Declared `Set -> Set -> Prop`, but the body is a bare sort.
    r.set(
        iri("urn:eigenius:eigentt:definition_body"),
        encode_type(&Exp::Sort(0)).unwrap(),
    );
    let errs = definition_errors(r);
    assert!(
        errs.iter().any(|m| m.contains("does not inhabit")),
        "a body of the wrong type must be refused; got {errs:?}"
    );
}

// ── D66 slice 2: the ESL `def` surface, end to end ─────────────────────────────────────────────

/// `def` compiles, commits, and unfolds — lexer → parser → compiler → decode in one pass.
#[test]
fn esl_def_compiles_and_unfolds_at_a_use_site() {
    let src = r#"
        namespace ont = "urn:eigenius:ontology";
        namespace d   = "urn:eigenius:demo:esl";
        def d:Activity(g : Set, a : Set) : Prop =
            ont:prep_of(ont:kind_of(g), ont:kind_of(a))
            desc: "the a-activity of g";
    "#;
    let resources = esl::compile(src).expect("`def` compiles");
    let def = resources
        .iter()
        .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:demo:esl:Activity"))
        .expect("the definition resource is emitted");
    assert!(
        def.is_a()
            .iter()
            .any(|c| c.as_str() == "urn:eigenius:eigentt:Definition"),
        "a `def` must mint an eigentt:Definition, got {:?}",
        def.is_a()
    );
    assert!(def
        .get(&iri("urn:eigenius:eigentt:definition_type"))
        .is_some());
    assert!(def
        .get(&iri("urn:eigenius:eigentt:definition_body"))
        .is_some());

    // Commit it onto a chain that has the vocabulary, and check a use unfolds.
    let mut b = LayerBuilder::new("esl-def", Some(chain_with_parse_vocabulary()));
    for r in resources {
        b.add_resource(r).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let use_site = app2(
        Exp::EigonAxiom(iri("urn:eigenius:demo:esl:Activity")),
        cls("urn:eigenius:demo:cls:WRN"),
        cls("urn:eigenius:demo:cls:exonuclease"),
    );
    let decoded = decode_type(&encode_type(&use_site).unwrap(), &layer).expect("the use decodes");
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
    assert_eq!(
        decoded, expected,
        "a use of an ESL-authored definition must unfold to its body"
    );
}

/// The printer emits generic `resource X : Class { … }` blocks rather than `def` syntax — the same
/// treatment `axiom` gets. That is fine *provided it round-trips*: `decompile --verify` recompiles
/// what it printed and compares. Verify it here rather than assume it, since a definition that
/// prints but does not reparse would break `decompile` the moment a chain holds one.
#[test]
fn a_definition_round_trips_through_the_printer() {
    let src = r#"
        namespace ont = "urn:eigenius:ontology";
        namespace d   = "urn:eigenius:demo:esl";
        def d:Activity(g : Set, a : Set) : Prop =
            ont:prep_of(ont:kind_of(g), ont:kind_of(a));
    "#;
    let original = esl::compile(src).expect("compiles");
    // The printer takes an Eigon-JSON document, the same path `eigenius decompile` uses.
    let doc = eigenius_kernel::ontology::eigon_json::serialize_document(&original);
    let printed = eigenius_kernel::esl::print::print_document(&doc).expect("prints");
    let reparsed = esl::compile(&printed)
        .unwrap_or_else(|e| panic!("printed ESL does not recompile: {e:?}\n---\n{printed}"));

    let find = |rs: &[Resource]| {
        rs.iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some("urn:eigenius:demo:esl:Activity"))
            .cloned()
            .expect("definition present")
    };
    let (a, b) = (find(&original), find(&reparsed));
    for prop in [
        "urn:eigenius:eigentt:definition_type",
        "urn:eigenius:eigentt:definition_body",
    ] {
        assert_eq!(
            a.get(&iri(prop)),
            b.get(&iri(prop)),
            "`{prop}` must survive print → recompile"
        );
    }
}

/// **Does δ compose?** One definition's body may reference another, and the stored body keeps that
/// reference *folded* — nothing normalizes it away at commit (Rule 24 checks β only). That is only
/// sound if decode unfolds nested definitions recursively, so both ends of the witness key still
/// land on the same fully-unfolded term.
#[test]
fn nested_definitions_unfold_all_the_way_at_decode() {
    let src = r#"
        namespace ont = "urn:eigenius:ontology";
        namespace d   = "urn:eigenius:demo:esl";
        def d:Inner(x : Set) : lexicon:Entity = ont:kind_of(x);
        def d:Outer(g : Set, a : Set) : Prop  = ont:prep_of(d:Inner(g), d:Inner(a));
    "#;
    let src = src.replace(
        "namespace ont",
        "namespace lexicon = \"urn:eigenius:lexicon\";\n        namespace ont",
    );
    let resources = esl::compile(&src).expect("nested defs compile");
    let mut b = LayerBuilder::new("nested", Some(chain_with_parse_vocabulary()));
    for r in resources {
        b.add_resource(r).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let use_site = app2(
        Exp::EigonAxiom(iri("urn:eigenius:demo:esl:Outer")),
        cls("urn:eigenius:demo:cls:WRN"),
        cls("urn:eigenius:demo:cls:exonuclease"),
    );
    let decoded = decode_type(&encode_type(&use_site).unwrap(), &layer).expect("decodes");

    // Fully unfolded: BOTH levels gone, `Inner` nowhere in the result.
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
    assert_eq!(
        decoded, expected,
        "decode must unfold nested definitions all the way, or a folded inner reference would \
         survive into the hash on one side and not the other"
    );
}

// ── D66 slice 2: the gate items I had not actually verified ────────────────────────────────────

/// Gate item: a **partial** application decodes to a β-normal `Lam`, not to a redex.
///
/// Peel-and-substitute stops when arguments run out. Claimed in D66 §2.4; never tested until now.
#[test]
fn a_partial_application_decodes_to_a_beta_normal_lambda() {
    let layer = chain_with_definition(DEF, false);
    // `F(WRN)` — one argument to a two-parameter definition.
    let partial = Exp::App(
        Box::new(Exp::EigonAxiom(iri(DEF))),
        Box::new(cls("urn:eigenius:demo:cls:WRN")),
    );
    let decoded = decode_type(&encode_type(&partial).unwrap(), &layer).expect("decodes");
    match &decoded {
        Exp::Lam(..) => {}
        other => panic!("a partial application must leave a Lam, got {other:?}"),
    }
    fn has_redex(e: &Exp) -> bool {
        match e {
            Exp::App(f, a) => matches!(f.as_ref(), Exp::Lam(..)) || has_redex(f) || has_redex(a),
            Exp::Lam(_, b) => has_redex(b),
            Exp::Fst(x) | Exp::Snd(x) => has_redex(x),
            Exp::Sig(_, d, b) | Exp::Pi(_, d, b) => has_redex(d) || has_redex(b),
            _ => false,
        }
    }
    assert!(!has_redex(&decoded), "still no redex: {decoded:?}");
}

/// An ESL-authored `def` passes commit validation — including Rule 24. My earlier Rule 24 tests
/// used a hand-built resource; this checks the one the compiler actually emits.
#[test]
fn an_esl_authored_def_passes_commit_validation() {
    let src = r#"
        namespace ont = "urn:eigenius:ontology";
        namespace d   = "urn:eigenius:demo:esl";
        def d:Activity(g : Set, a : Set) : Prop =
            ont:prep_of(ont:kind_of(g), ont:kind_of(a));
    "#;
    let base = chain_with_parse_vocabulary();
    let validator = Validator::new(Arc::clone(&base));
    for r in esl::compile(src).expect("compiles") {
        let errs: Vec<String> = validator
            .validate_resource(&r)
            .into_iter()
            .map(|e| format!("{:?}: {}", e.rule, e.message))
            .collect();
        assert!(
            errs.is_empty(),
            "compiler-emitted resource {:?} failed validation: {errs:?}",
            r.id()
        );
    }
}

/// A **proposition that uses a definition** passes commit validation.
///
/// This is the question behind "does a transparent definition need an `axiom_env` entry?" Rule 21
/// requires every `eigentt:TypeExpr`-valued property to decode *and* type-check. If a definition's
/// IRI reached `check_infer` as a bare constant it would have no registered type and fail — so this
/// passing is the evidence that decode unfolds it first and the checker never sees the name.
#[test]
fn a_proposition_using_a_definition_type_checks_at_commit() {
    let src = r#"
        namespace ont = "urn:eigenius:ontology";
        namespace d   = "urn:eigenius:demo:esl";
        def d:Activity(g : Set, a : Set) : Prop =
            ont:prep_of(ont:kind_of(g), ont:kind_of(a));
    "#;
    let mut b = LayerBuilder::new("defs", Some(chain_with_parse_vocabulary()));
    for r in esl::compile(src).expect("compiles") {
        b.add_resource(r).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    // A resource whose canonical_proposition is a USE of the definition.
    let mut claim = Resource::new(iri("urn:eigenius:demo:esl:claim"));
    claim.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE))]),
    );
    claim.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("test".into()),
    );
    claim.set(
        iri(wk::CANONICAL_PROPOSITION),
        encode_type(&app2(
            Exp::EigonAxiom(iri("urn:eigenius:demo:esl:Activity")),
            cls("urn:eigenius:demo:cls:WRN"),
            cls("urn:eigenius:demo:cls:exonuclease"),
        ))
        .unwrap(),
    );

    let errs: Vec<String> = Validator::new(layer)
        .validate_resource(&claim)
        .into_iter()
        .map(|e| format!("{:?}: {}", e.rule, e.message))
        .collect();
    assert!(
        errs.is_empty(),
        "a proposition citing a definition must type-check at commit; got {errs:?}"
    );
}

/// **Why exempting `definition_body` from Rule 21 is sound.**
///
/// Rule 21 ends in `check_infer`, and a lambda chain has no inferable type, so for a definition body
/// the rule contributes nothing but a spurious rejection. The body is instead checked by Rule 24
/// against the declared `definition_type` — the correct mode, and strictly stronger.
///
/// That argument has a hole unless one thing holds: the exemption is keyed on the *property IRI*,
/// not on the class, so it would be an escape hatch if `definition_body` could ride on a resource
/// that is NOT an `eigentt:Definition` — Rule 24 would not run, Rule 21 would be exempt, and the
/// value would be checked by nothing.
///
/// It cannot. Rule 10 is restrictive and `definition_body`'s `core:domain` is `[eigentt:Definition]`.
/// This test is what makes that a checked fact rather than a reading of the ontology.
#[test]
fn definition_body_cannot_escape_checking_by_riding_on_another_class() {
    let layer = chain_with_parse_vocabulary();
    let mut smuggler = Resource::new(iri("urn:eigenius:demo:esl:smuggler"));
    smuggler.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::DECLARED_RESOURCE))]),
    );
    smuggler.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("test".into()),
    );
    // A body that Rule 24 would reject (ill-typed), on a resource Rule 24 will never look at.
    smuggler.set(
        iri("urn:eigenius:eigentt:definition_body"),
        encode_type(&Exp::Sort(0)).unwrap(),
    );

    let errs = Validator::new(layer).validate_resource(&smuggler);
    assert!(
        errs.iter()
            .any(|e| e.rule == ValidationRule::DomainViolation),
        "`definition_body` outside an eigentt:Definition must be refused by the domain rule, or the \
         Rule 21 exemption becomes an escape hatch; got {:?}",
        errs.iter().map(|e| format!("{:?}", e.rule)).collect::<Vec<_>>()
    );
}

// ── D66 slice 3 crux: does a definition unfold to EXACTLY the committed parse? ─────────────────

/// The lexicon vocabulary `demo/prose-to-formulas` parses into. Stand-ins carrying the real IRIs —
/// decode only needs each to resolve to something of the right class, which keeps this test free of
/// the WordNet/UMLS snapshot. Both sides of every comparison below reference the same IRIs and go
/// through the same decode, so the equality result is unaffected by the substitution.
const LEXICON_STANDINS: &str = r#"
    namespace lexicon = "urn:eigenius:lexicon";
    namespace wn      = "urn:eigenius:wn";
    namespace umlscui = "urn:eigenius:umlscui";
    class wn:n13440063 { }
    class wn:n14606137 { }
    class wn:n05890249 { }
    class wn:n14239918 { }
    class umlscui:C0388246 { }
    class umlscui:C0920269 { }
    class umlscui:C0920283 { }
    axiom wn:v02203362_t : lexicon:Entity -> lexicon:Entity -> Prop
    axiom wn:v02627934_t : lexicon:Entity -> lexicon:Entity -> Prop
"#;

/// The MSI-cancer-model term, shared by both sentences. Not a class — a nested compound kind.
const MSI: &str = r#"(exists x0 : wn:n05890249 =>
                        ontology:compound_kind(
                            x0,
                            (exists x1 : wn:n14239918 =>
                                ontology:compound_kind(x1, umlscui:C0920269))))"#;

/// `demo/prose-to-formulas/claims-intact.esl` commits each sentence's parse. D66 replaces the
/// generated shape rules with definitions, so a use of `HasActivity` / `RequiresActivity` must
/// decode to **exactly** the parse — not equivalently: the witness key hashes the term, so any
/// difference and `inference.esl` finds no witness.
///
/// Checked against the committed artifact for both sentences, offline, before anything downstream
/// depends on it.
fn definition_matches_committed_parse(verb_axiom: &str, activity: &str, def_name: &str) {
    // (a) The parse, as `claims-*.esl` stores it.
    let parse_src = format!(
        r#"{LEXICON_STANDINS}
        namespace ontology = "urn:eigenius:ontology";
        namespace logic    = "urn:eigenius:logic";
        namespace eigentt  = "urn:eigenius:eigentt";
        namespace reflection = "urn:eigenius:reflection";
        namespace p = "urn:eigenius:demo:parse";
        resource p:claim : reflection:DeclaredResource {{
            reflection:declared_by = "test";
            reflection:canonical_proposition = type_expr(
                {verb_axiom}(
                    eigentt:fst(ontology:the(
                        (exists x0 : wn:n13440063 =>
                            logic:And(
                                ontology:compound_kind(x0, {activity}),
                                ontology:prep_of(x0, ontology:kind_of(umlscui:C0388246))
                            )))),
                    ontology:kind_of({MSI})
                )
            );
        }}"#
    );

    // (b) The definition, and a call at the same three positions.
    let def_src = format!(
        r#"{LEXICON_STANDINS}
        namespace ontology = "urn:eigenius:ontology";
        namespace logic    = "urn:eigenius:logic";
        namespace eigentt  = "urn:eigenius:eigentt";
        namespace reflection = "urn:eigenius:reflection";
        namespace onco = "urn:eigenius:demo:onco";
        namespace d = "urn:eigenius:demo:def";

        def onco:{def_name}(m : Set, g : Set, a : Set) : Prop =
            {verb_axiom}(
                eigentt:fst(ontology:the(
                    (exists x0 : wn:n13440063 =>
                        logic:And(
                            ontology:compound_kind(x0, a),
                            ontology:prep_of(x0, ontology:kind_of(g))
                        )))),
                ontology:kind_of(m));

        resource d:claim : reflection:DeclaredResource {{
            reflection:declared_by = "test";
            reflection:canonical_proposition = type_expr(
                onco:{def_name}({MSI}, umlscui:C0388246, {activity})
            );
        }}"#
    );

    let base = chain_with_parse_vocabulary();
    let build = |src: &str| {
        let mut b = LayerBuilder::new("case", Some(Arc::clone(&base)));
        let rs = esl::compile(src).unwrap_or_else(|e| panic!("compiles: {e:?}"));
        for r in rs.clone() {
            b.add_resource(r).unwrap();
        }
        (Arc::new(b.build(LayerStorage::in_memory())), rs)
    };
    let (parse_layer, parse_rs) = build(&parse_src);
    let (def_layer, def_rs) = build(&def_src);

    let prop_of = |rs: &[Resource], id: &str| {
        rs.iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some(id))
            .and_then(|r| r.get(&iri(wk::CANONICAL_PROPOSITION)).cloned())
            .expect("claim carries a proposition")
    };
    let parse_stored = prop_of(&parse_rs, "urn:eigenius:demo:parse:claim");
    let call_stored = prop_of(&def_rs, "urn:eigenius:demo:def:claim");
    assert_ne!(
        parse_stored, call_stored,
        "{def_name}: the STORED forms must differ — folded call vs spelled-out parse"
    );

    let parse_decoded = decode_type(&parse_stored, &parse_layer).expect("parse decodes");
    let call_decoded = decode_type(&call_stored, &def_layer).expect("call decodes");
    assert_eq!(
        call_decoded, parse_decoded,
        "{def_name} must unfold to EXACTLY the committed parse"
    );
    assert_eq!(
        hash_proposition_exp(&call_decoded).unwrap(),
        hash_proposition_exp(&parse_decoded).unwrap(),
        "{def_name}: and therefore hash identically — this is what makes the lift free"
    );
}

/// Sentence 1 — «MSI cancer models had the exonuclease activity of WRN» (`claim_1`).
#[test]
fn has_activity_unfolds_to_exactly_the_committed_parse() {
    definition_matches_committed_parse("wn:v02203362_t", "wn:n14606137", "HasActivity");
}

/// Sentence 2 — «MSI cancer models required the helicase activity of WRN» (`claim_2`).
///
/// `inference.esl` concludes this proposition and `claim_2` asserts it independently; the demo's
/// "justified twice" turns on them being the same term.
#[test]
fn requires_activity_unfolds_to_exactly_the_committed_parse() {
    definition_matches_committed_parse("wn:v02627934_t", "umlscui:C0920283", "RequiresActivity");
}
