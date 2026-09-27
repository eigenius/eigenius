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

//! D95 slices 4 and 5 — measure phrases in the grammar and their consumers. The categories
//! (`cat_mp`, `cat_unit_forall`, `lexicon:Reading`), the `Difference` type, the unit-polymorphic
//! application, the seeding and the prepositions over a measured value (`closed-class.esl`) are in
//! the bootstrap chain; the fixture adds the content words around them, and a verb that takes only a
//! difference in kelvin, which slice 7's consumers will be shaped as. No DB, no reseed.

use std::sync::Arc;

use eigenius_kernel::dcg::{entry_to_item, is_ctor, pretty_term, Identity, Parser};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;

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

axiom lexicon:received : lexicon:Entity -> lexicon:Entity -> Prop
resource lexicon:received_v : lexicon:LexicalEntry {
    lexicon:form     = "received";
    lexicon:cat      = type_expr( lexicon:fwd(lexicon:m_all, lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:fin), lexicon:cat_np(lexicon:Entity, lexicon:num_any)), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:received;
    lexicon:sem_type = type_expr( lexicon:Entity -> lexicon:Entity -> Prop );
    lexicon:sense    = "received";
}

class lexicon:Dose : lexicon:Entity { }
resource lexicon:dose_n : lexicon:LexicalEntry {
    lexicon:form     = "dose";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Dose, lexicon:num_any) );
    lexicon:sem      = lexicon:Dose;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "dose";
}
class lexicon:Etoposide : lexicon:Entity { }
resource lexicon:etoposide_n : lexicon:LexicalEntry {
    lexicon:form     = "etoposide";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Etoposide, lexicon:mass) );
    lexicon:sem      = lexicon:Etoposide;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "etoposide";
}
class lexicon:Temperature : lexicon:Entity { }
resource lexicon:temperature_n : lexicon:LexicalEntry {
    lexicon:form     = "temperature";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Temperature, lexicon:num_any) );
    lexicon:sem      = lexicon:Temperature;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "temperature";
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
        r[0].contains("ontology:prep_at_value")
            && r[0].contains("numer: 6203")
            && r[0].contains("denom: 20"),
        "{r:#?}"
    );
    assert!(!r[0].contains("compound_kind"), "{r:#?}");
}

/// Each VP-adjunct preposition over a measured value, on a sentence of the WRN methods' shape: the
/// relation, and the value in base units (`2 h` is 7200 s, `50 μl` is 1/20000 m³).
#[test]
fn each_preposition_takes_a_measured_value() {
    let parser = Parser::build(layer());
    for (text, relation, magnitude) in [
        (
            "HeLa incubated for 2 h",
            "prep_for_value",
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated after 72 h",
            "prep_after_value",
            "numer: 259200, denom: 1",
        ),
        (
            "HeLa incubated in 50 μl",
            "prep_in_value",
            "numer: 1, denom: 20000000",
        ),
        (
            "HeLa incubated with 10%",
            "prep_with_value",
            "numer: 1, denom: 10",
        ),
        (
            "HeLa incubated at 37 °C for 2 h",
            "prep_for_value",
            "numer: 7200, denom: 1",
        ),
    ] {
        let r = readings(&parser, text);
        assert_eq!(r.len(), 1, "{text}: {r:#?}");
        assert!(
            r[0].contains(&format!("ontology:{relation}")) && r[0].contains(magnitude),
            "{text}: {r:#?}"
        );
    }
}

/// `after` is a closed-class preposition over an NP as well as over a measured value.
#[test]
fn after_takes_an_np() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa incubated after the dose");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(
        r[0].contains("ontology:prep_after\"") && r[0].contains("lexicon:Dose"),
        "{r:#?}"
    );
}

/// A noun takes a measured value post-nominally: `a dose of 5 mg/kg`.
#[test]
fn a_noun_takes_a_measured_value() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa received a dose of 5 mg/kg");
    assert!(!r.is_empty());
    assert!(
        r.iter()
            .all(|s| s.contains("ontology:prep_of_value") && s.contains("numer: 1, denom: 200000")),
        "{r:#?}"
    );
}

/// A quantity before a noun modifies it, and after the copula it is predicated:
/// `has_quantity(x, u, q)` either way.
#[test]
fn a_prenominal_and_a_predicative_measure_phrase() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa incubated with 10 μM etoposide");
    assert!(!r.is_empty());
    assert!(
        r.iter().all(|s| s.contains("ontology:has_quantity")
            && s.contains("lexicon:Etoposide")
            && s.contains("numer: 1, denom: 100")),
        "{r:#?}"
    );
    let r = readings(&parser, "the temperature was 37 °C");
    assert!(!r.is_empty());
    assert!(
        r.iter()
            .all(|s| s.contains("ontology:has_quantity") && s.contains("numer: 6203")),
        "{r:#?}"
    );
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
        "HeLa incubated at 37 °C for 2 h",
        "HeLa incubated with 10 μM etoposide",
        "HeLa received a dose of 5 mg/kg",
        "the temperature was 37 °C",
    ] {
        assert_eq!(readings(&packed, text), readings(&unpacked, text), "{text}");
    }
}

/// Every `cat_mp(unit, reading)` inside a category.
fn measure_phrases<'e>(cat: &'e Exp, out: &mut Vec<&'e [Exp]>) {
    if let Some(args) = is_ctor(cat, "cat_mp") {
        out.push(args);
    }
    match cat {
        Exp::InductiveCtor(_, _, args) => args.iter().for_each(|a| measure_phrases(a, out)),
        Exp::Lam(_, body) => measure_phrases(body, out),
        Exp::App(f, a) => {
            measure_phrases(f, out);
            measure_phrases(a, out);
        }
        _ => {}
    }
}

/// Decision 5: every closed-class consumer of a measure phrase names the reading it takes. One
/// whose reading were a variable would take the value and the difference items alike, and
/// `at 37 °C` would be 310.15 K and 37 K at once. The 33 prepositions of slice 5 take values.
#[test]
fn every_consumer_names_its_reading() {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let head = ctx.head();
    let mut layer = head;
    while layer.name() != "closed-class" {
        layer = layer.parent().expect("closed-class is in the chain");
    }
    let entry = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    let mut readings: Vec<String> = Vec::new();
    for (iri, r) in layer.iter_resources() {
        if !r.is_instance_of(&entry) {
            continue;
        }
        let item = entry_to_item(head, &r).unwrap_or_else(|e| panic!("{iri}: {e}"));
        let mut mps = Vec::new();
        measure_phrases(item.cat(), &mut mps);
        for args in mps {
            match &args[1] {
                Exp::InductiveCtor(_, reading, rest) if rest.is_empty() => {
                    readings.push(reading.clone())
                }
                other => panic!("{iri} takes a measure phrase at reading {other:?}"),
            }
        }
    }
    assert_eq!(readings.len(), 33, "{readings:?}");
    assert!(readings.iter().all(|r| r == "value"), "{readings:?}");
}
