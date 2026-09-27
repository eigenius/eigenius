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

//! D95 slice 4 — measure phrases in the grammar. The categories (`cat_mp`, `cat_unit_forall`,
//! `lexicon:Reading`), the `Difference` type, the unit-polymorphic application and the seeding are
//! committed; the fixture adds consumers shaped as slice 5's will be — a VP-adjunct `at` that takes a
//! measured value of any unit, and a verb that takes only a difference in kelvin — plus the words
//! around them. No DB, no reseed.

use std::sync::Arc;

use eigenius_kernel::dcg::{pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};

const FIXTURE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
namespace logic   = "urn:eigenius:logic";
namespace core    = "urn:eigenius:core";
namespace units   = "urn:eigenius:units";

axiom lexicon:hela : lexicon:Entity
resource lexicon:hela_sem : lexicon:SemTerm { lexicon:term = type_expr( lexicon:hela ); }
resource lexicon:hela_np : lexicon:LexicalEntry {
    lexicon:form = "HeLa"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem = lexicon:hela_sem; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "hela";
}

// A digit-initial identifier the lexicon knows (decision 6): `5A` is a word as well as 5 A.
axiom lexicon:mccoys_5a : lexicon:Entity
resource lexicon:mccoys_5a_sem : lexicon:SemTerm { lexicon:term = type_expr( lexicon:mccoys_5a ); }
resource lexicon:mccoys_5a_np : lexicon:LexicalEntry {
    lexicon:form = "5A"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem = lexicon:mccoys_5a_sem; lexicon:sem_type = type_expr( lexicon:Entity );
    lexicon:sense = "mccoys_5a";
}

axiom lexicon:incubated : lexicon:Entity -> Prop
resource lexicon:incubated_v : lexicon:LexicalEntry {
    lexicon:form     = "incubated";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:incubated;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "incubated";
}

// `at` + a measured value of any unit: ((S\NP)\(S\NP))/MP[u, value] under the unit binder.
axiom lexicon:at_value : forall (u : core:unit) => lexicon:Entity -> units:Quantity(u) -> Prop
resource lexicon:at_value_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (u : core:unit) => fun (q : units:Quantity(u)) => fun (V : lexicon:Entity -> Prop) => fun (s : lexicon:Entity) =>
            logic:And(V(s), lexicon:at_value(u, s, q))
          : forall (u : core:unit) => units:Quantity(u) -> (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop) )
    );
}
resource lexicon:at_value_adjunct : lexicon:LexicalEntry {
    lexicon:form     = "at";
    lexicon:cat      = type_expr( lexicon:cat_unit_forall(fun (u : core:unit) => lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any))), lexicon:cat_mp(u, lexicon:value))) );
    lexicon:sem      = lexicon:at_value_sem;
    lexicon:sem_type = type_expr( forall (u : core:unit) => units:Quantity(u) -> (lexicon:Entity -> Prop) -> (lexicon:Entity -> Prop) );
    lexicon:sense    = "at.value";
}

// `rose` + a DIFFERENCE in kelvin: a consumer that names its dimension and its reading.
axiom lexicon:rose_by : lexicon:Entity -> units:Difference(u"K") -> Prop
resource lexicon:rose_sem : lexicon:SemTerm {
    lexicon:term = type_expr(
        ( fun (d : units:Difference(u"K")) => fun (x : lexicon:Entity) => lexicon:rose_by(x, d)
          : units:Difference(u"K") -> lexicon:Entity -> Prop )
    );
}
resource lexicon:rose_v : lexicon:LexicalEntry {
    lexicon:form     = "rose";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_mp(u"K", lexicon:difference)) );
    lexicon:sem      = lexicon:rose_sem;
    lexicon:sem_type = type_expr( units:Difference(u"K") -> lexicon:Entity -> Prop );
    lexicon:sense    = "rose";
}
"#;

fn layer() -> Arc<Layer> {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let resources = esl::compile(FIXTURE, ctx.head()).expect("fixture compiles");
    let mut b = LayerBuilder::new("quantities", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add fixture resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// The closed readings of `text`, sorted: each as the sem's debug form, which shows the literals
/// (`numer: 6203, denom: 20`, `Unit(K)`) that `pretty_term` prints as `<term>`, then `pretty_term`'s
/// form for the failure message.
fn readings(parser: &Parser, text: &str) -> Vec<String> {
    let mut out: Vec<String> = parser
        .parse(text, &Identity)
        .iter()
        .map(|it| format!("{:?}  ⟨{}⟩", it.sem(), pretty_term(it.sem())))
        .collect();
    out.sort();
    out
}

/// A consumer that takes a VALUE composes with the value item only: `37 °C` is 310.15 K, once.
#[test]
fn a_value_consumer_takes_the_value_reading() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa incubated at 37 °C");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(
        r[0].contains("at_value") && r[0].contains("numer: 6203") && r[0].contains("denom: 20"),
        "{r:#?}"
    );
    assert!(!r[0].contains("compound_kind"), "{r:#?}");
}

/// `931g` seeds gram and standard gravity; a unit-polymorphic consumer takes both, and the reading
/// choice is left two readings (D93, `enc:DecisionPoint`).
#[test]
fn an_ambiguous_unit_is_two_readings() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa incubated at 931g");
    assert_eq!(r.len(), 2, "{r:#?}");
    assert!(
        r.iter()
            .any(|s| s.contains("numer: 931,") && s.contains("denom: 1000")),
        "{r:#?}"
    );
    assert!(r.iter().any(|s| s.contains("numer: 182599823")), "{r:#?}");
}

/// A consumer that names its dimension and reading: `rose 5 °C` is a difference of 5 K, without the
/// offset; `rose 5 mg` has no parse. (The unit is in the item's category and the term's annotation,
/// which normalisation erases, so the sem shows the magnitude.)
#[test]
fn a_difference_consumer_names_its_dimension() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa rose 5 °C");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(
        r[0].contains("rose_by")
            && r[0].contains("mk_difference")
            && r[0].contains("numer: 5, denom: 1")
            && !r[0].contains("numer: 5563"),
        "{r:#?}"
    );
    assert!(readings(&parser, "HeLa rose 5 mg").is_empty());
}

/// Decision 6: an attached quantity keeps the word it spells. `5A` is 5 A and the medium the fixture
/// names; as a subject only the word composes.
#[test]
fn an_attached_quantity_keeps_its_word_reading() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "5A incubated");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(r[0].contains("mccoys_5a"), "{r:#?}");
}

/// The packed forest keys a unit exactly, so it reads what the unpacked chart reads.
#[test]
fn packed_equals_unpacked_on_quantities() {
    let layer = layer();
    let packed = Parser::build(Arc::clone(&layer));
    let unpacked = Parser::build(layer).with_packing(false);
    for text in [
        "HeLa incubated at 37 °C",
        "HeLa incubated at 931g",
        "HeLa rose 5 °C",
        "HeLa rose 5 mg",
        "HeLa incubated at 2 h",
    ] {
        assert_eq!(readings(&packed, text), readings(&unpacked, text), "{text}");
    }
}
