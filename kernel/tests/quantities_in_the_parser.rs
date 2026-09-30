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

//! D95 slices 4 to 7 — measure phrases in the grammar, their consumers, bounds and ranges on them,
//! and counts. The categories
//! (`cat_mp`, `cat_unit_forall`, `lexicon:Reading`), the `Difference` type, the unit-polymorphic
//! application, the seeding and the prepositions over a measured value (`closed-class.esl`) are in
//! the bootstrap chain; the fixture adds the content words around them, and a verb that takes only a
//! difference in kelvin, which slice 7's consumers will be shaped as. No DB, no reseed.

use std::sync::Arc;

use eigenius_kernel::dcg::{
    apply, entry_to_item, is_ctor, pretty_term, Identity, Item, Parser, RightContext,
};
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
class lexicon:Cell : lexicon:Entity { }
resource lexicon:cells_n : lexicon:LexicalEntry {
    lexicon:form     = "cells";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Cell, lexicon:pl) );
    lexicon:sem      = lexicon:Cell;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "cells";
}
resource lexicon:cell_n : lexicon:LexicalEntry {
    lexicon:form     = "cell";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Cell, lexicon:sg) );
    lexicon:sem      = lexicon:Cell;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "cell";
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
        "HeLa incubated at less than 37 °C",
        "HeLa received a dose of at least 5 mg/kg",
        "the temperature was less than 37 °C",
        "HeLa incubated with less than 10 μM etoposide",
        "HeLa incubated for less than about 2 h",
        "HeLa incubated with >90% etoposide",
        "the temperature < 37 °C",
        "the temperature = 3.1 × 10² K",
        "HeLa incubated for 2–3 h",
        "HeLa incubated with 80–90% etoposide",
        "HeLa incubated at 37 °C or higher",
        "the temperature was 4 °C or lower",
        "two cells incubated",
        "at least 1,000 cells incubated",
        "HeLa received 4–7 cells",
        "more than half of the cells incubated",
        "15% of cells incubated",
        "HeLa incubated in more than half of the cells",
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
/// `at 37 °C` would be 310.15 K and 37 K at once. The 33 prepositions of slice 5, the 18 word-marker
/// entries of slice 6a, the 19 symbol entries of 6b, the 8 postfix entries of 6d, and `half` and the
/// two partitive `of`s of 7c take values.
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
    assert_eq!(readings.len(), 81, "{readings:?}");
    assert!(readings.iter().all(|r| r == "value"), "{readings:?}");
}

/// Whether a bound's constraint `rel` puts the quantified value first (`lt(u, q, b)`: below the
/// bound) or the bound first (`lt(u, b, q)`: above it), read off the pretty form, where the value is
/// a bound variable `G#n` and the bound a literal `mk_quantity(…)`.
fn value_first(pretty: &str, rel: &str) -> Option<bool> {
    let at = pretty.find(&format!("{rel}(<term>, "))? + rel.len() + "(<term>, ".len();
    Some(pretty[at..].starts_with("G#"))
}

/// Slice 6a: a bound constrains the value a consumer takes (D95 implementation plan, slice 6). The
/// value is quantified, `∃q. C(q) ∧ …`, and the constraint's direction is its argument order:
/// `less than b` is `lt(q, b)`, `more than b` is `lt(b, q)`. `37 °C` is still the value 310.15 K.
#[test]
fn a_bound_constrains_the_value_a_consumer_takes() {
    let parser = Parser::build(layer());
    for (text, relation, constraint, value_first_expected, magnitude) in [
        (
            "HeLa incubated at less than 37 °C",
            "prep_at_value",
            "lt",
            true,
            "numer: 6203, denom: 20",
        ),
        (
            "HeLa incubated for more than 2 h",
            "prep_for_value",
            "lt",
            false,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated for at least 2 h",
            "prep_for_value",
            "le",
            false,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated for at most 2 h",
            "prep_for_value",
            "le",
            true,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated for up to 2 h",
            "prep_for_value",
            "le",
            true,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated at approximately 37 °C",
            "prep_at_value",
            "approx",
            true,
            "numer: 6203, denom: 20",
        ),
        (
            "HeLa incubated for about 2 h",
            "prep_for_value",
            "approx",
            true,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated for around 2 h",
            "prep_for_value",
            "approx",
            true,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated for roughly 2 h",
            "prep_for_value",
            "approx",
            true,
            "numer: 7200, denom: 1",
        ),
        (
            "HeLa incubated with more than 10%",
            "prep_with_value",
            "lt",
            false,
            "numer: 1, denom: 10",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains(&format!("ontology:{relation}")),
            "{text}: {pretty}"
        );
        assert!(
            debug.contains(&format!("units:{constraint}")),
            "{text}: {pretty}"
        );
        assert!(debug.contains(magnitude), "{text}: {debug}");
        assert_eq!(
            value_first(&pretty, constraint),
            Some(value_first_expected),
            "{text}: {pretty}"
        );
    }
}

/// Slice 6a: a noun modifier, the copula and a prenominal modifier take a bound as they take a
/// measured value — the marker's predicate entry is `λx. ∃q. C(q) ∧ has_quantity(x, u, q)`.
#[test]
fn a_bound_in_each_position() {
    let parser = Parser::build(layer());
    for (text, relation, constraint, value_first_expected, noun) in [
        (
            "HeLa received a dose of at least 5 mg/kg",
            "prep_of_value",
            "le",
            false,
            "lexicon:Dose",
        ),
        (
            "the temperature was less than 37 °C",
            "has_quantity",
            "lt",
            true,
            "lexicon:Temperature",
        ),
        (
            "HeLa incubated with less than 10 μM etoposide",
            "has_quantity",
            "lt",
            true,
            "lexicon:Etoposide",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains(&format!("ontology:{relation}"))
                && debug.contains(&format!("units:{constraint}"))
                && debug.contains(noun),
            "{text}: {pretty}"
        );
        assert_eq!(
            value_first(&pretty, constraint),
            Some(value_first_expected),
            "{text}: {pretty}"
        );
    }
}

/// A marker is a consumer too: `less than about 2 h` is a value below one of about 2 h.
#[test]
fn a_bound_takes_a_bound() {
    let parser = Parser::build(layer());
    let parsed = parser.parse("HeLa incubated for less than about 2 h", &Identity);
    assert_eq!(parsed.len(), 1);
    let debug = format!("{:?}", parsed[0].sem());
    assert!(
        debug.contains("units:approx")
            && debug.contains("units:lt")
            && debug.contains("prep_for_value"),
        "{}",
        pretty_term(parsed[0].sem())
    );
}

/// Every marker takes a value (decision 2), so a consumer of a difference takes no bound: `rose`
/// takes a difference in kelvin and `rose less than 5 °C` has no parse. Bounds on differences wait
/// for an attested one.
#[test]
fn a_bound_on_a_difference_is_not_built() {
    let parser = Parser::build(layer());
    assert!(readings(&parser, "HeLa rose less than 5 °C").is_empty());
    assert_eq!(readings(&parser, "HeLa rose 5 °C").len(), 1);
}

/// Decision 3: one rule serves every consumer. Each closed-class entry that takes a measure phrase
/// in kelvin on its right also takes a constraint in kelvin through `unit_constraint`, and yields the
/// category it yields for the measure phrase. (A postfix bound takes its value on the left; it makes a
/// constraint and takes none.)
#[test]
fn every_measure_consumer_takes_a_constraint() {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let head = ctx.head();
    let mut layer = head;
    while layer.name() != "closed-class" {
        layer = layer.parent().expect("closed-class is in the chain");
    }
    let cat_iri = Iri::parse("urn:eigenius:lexicon:Cat").unwrap();
    let reading_iri = Iri::parse("urn:eigenius:lexicon:Reading").unwrap();
    let kelvin = Exp::LitUnit(eigenius_kernel::units::Unit::parse_canonical("K").unwrap());
    let measure = |ctor: &str| {
        Exp::InductiveCtor(
            cat_iri.clone(),
            ctor.into(),
            vec![
                kelvin.clone(),
                Exp::InductiveCtor(reading_iri.clone(), "value".into(), vec![]),
            ],
        )
    };
    let point = Item::new(measure("cat_mp"), Exp::Var("q".into()));
    let bound = Item::new(measure("cat_mpc"), Exp::Var("c".into()));
    let entry = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    let mut consumers = 0;
    for (iri, r) in layer.iter_resources() {
        if !r.is_instance_of(&entry) {
            continue;
        }
        let consumer = entry_to_item(head, &r).unwrap_or_else(|e| panic!("{iri}: {e}"));
        let Some(with_point) = apply(&consumer, &point, head, RightContext::Other) else {
            continue;
        };
        let with_bound = apply(&consumer, &bound, head, RightContext::Other)
            .unwrap_or_else(|| panic!("{iri} takes 37 K and no bound in K"));
        assert_eq!(with_bound.cat(), with_point.cat(), "{iri}");
        consumers += 1;
    }
    assert_eq!(consumers, 70);
}

/// Slice 6a: a bound verbalizes as its words where the quantity would have rendered — after a
/// preposition, predicated, and nested — not as the existential it reads back as.
#[test]
fn a_bound_verbalizes_as_its_words() {
    use eigenius_kernel::dcg::{verbalize, Vb};
    let layer = layer();
    let parser = Parser::build(Arc::clone(&layer));
    let names = std::collections::BTreeMap::new();
    for (text, words) in [
        (
            "HeLa incubated at less than 37 °C",
            "at less than 6203/20 K",
        ),
        ("HeLa incubated for more than 2 h", "for more than 7200 s"),
        ("HeLa incubated for at least 2 h", "for at least 7200 s"),
        ("HeLa incubated for up to 2 h", "for at most 7200 s"),
        (
            "HeLa incubated at approximately 37 °C",
            "at about 6203/20 K",
        ),
        (
            "the temperature was less than 37 °C",
            "is less than 6203/20 K",
        ),
        (
            "HeLa received a dose of at least 5 mg/kg",
            "of at least 1/200000",
        ),
        (
            "HeLa incubated for less than about 2 h",
            "for less than about 7200 s",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}");
        let surface = verbalize(parsed[0].sem(), &Vb::surface(&names, &layer));
        assert!(surface.contains(words), "{text}: {surface}");
        assert!(!surface.contains("Quantity("), "{text}: {surface}");
    }
}

/// Slice 6b: a symbol is a bound, as its words are — before a noun (`>90% infection efficiency`) and
/// after a preposition.
#[test]
fn a_symbol_is_a_bound() {
    let parser = Parser::build(layer());
    for (text, relation, constraint, value_first_expected) in [
        (
            "HeLa incubated with >90% etoposide",
            "has_quantity",
            "lt",
            false,
        ),
        ("HeLa incubated at < 37 °C", "prep_at_value", "lt", true),
        ("HeLa incubated at ≤ 37 °C", "prep_at_value", "le", true),
        ("HeLa incubated at ≥ 37 °C", "prep_at_value", "le", false),
        ("HeLa incubated for ~ 2 h", "prep_for_value", "approx", true),
        ("HeLa incubated for ≈ 2 h", "prep_for_value", "approx", true),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains(&format!("ontology:{relation}"))
                && debug.contains(&format!("units:{constraint}")),
            "{text}: {pretty}"
        );
        assert_eq!(
            value_first(&pretty, constraint),
            Some(value_first_expected),
            "{text}: {pretty}"
        );
    }
}

/// Slice 6b: a symbol between a noun phrase and a value is a comparison clause, the predicate without
/// a copula; `=` states the value itself.
#[test]
fn a_symbol_between_a_noun_phrase_and_a_value_is_a_comparison() {
    let parser = Parser::build(layer());
    for (text, constraint, value_first_expected) in [
        ("the temperature < 37 °C", Some("lt"), true),
        ("the temperature > 37 °C", Some("lt"), false),
        ("the temperature ≥ 37 °C", Some("le"), false),
        ("the temperature ~ 37 °C", Some("approx"), true),
        ("the temperature = 37 °C", None, true),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains("ontology:has_quantity") && debug.contains("lexicon:Temperature"),
            "{text}: {pretty}"
        );
        assert!(debug.contains("numer: 6203, denom: 20"), "{text}: {debug}");
        match constraint {
            Some(c) => assert_eq!(
                value_first(&pretty, c),
                Some(value_first_expected),
                "{text}: {pretty}"
            ),
            None => assert!(
                ["lt", "le", "approx"]
                    .iter()
                    .all(|r| !debug.contains(&format!("urn:eigenius:units:{r}\""))),
                "{text}: {pretty}"
            ),
        }
    }
}

/// Slice 6b: scientific notation is one numeral, and a unit after it makes it a quantity:
/// `3.1 × 10² K` is 310 K, `2 × 10⁻³ mg/kg` is 2 × 10⁻⁹, `2 × 10⁻¹⁶` a bound in a comparison.
#[test]
fn scientific_notation_is_one_numeral() {
    let parser = Parser::build(layer());
    for (text, magnitude) in [
        ("the temperature = 3.1 × 10² K", "numer: 310, denom: 1"),
        ("the temperature = 3.1 x 10² K", "numer: 310, denom: 1"),
        (
            "HeLa incubated with 2 × 10⁻³ mg/kg",
            "numer: 1, denom: 500000000",
        ),
        (
            "the temperature < 2 × 10⁻¹⁶ K",
            "numer: 1, denom: 5000000000000000",
        ),
        (
            "the temperature < 2.2× 10-16 K",
            "numer: 11, denom: 50000000000000000",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        assert!(debug.contains(magnitude), "{text}: {debug}");
    }
}

/// Slice 6c: a range is a constraint, `lo ≤ q ≤ hi`, both endpoints in the unit written once after
/// the pair — after a preposition, before a noun and after the copula; an en-dash or a hyphen joins
/// it, and `37 °C` is still the value 310.15 K at either end.
#[test]
fn a_range_is_a_constraint() {
    let parser = Parser::build(layer());
    for (text, relation, low, high) in [
        (
            "HeLa incubated for 2–3 h",
            "prep_for_value",
            "numer: 7200, denom: 1",
            "numer: 10800, denom: 1",
        ),
        (
            "HeLa incubated for 2-3 h",
            "prep_for_value",
            "numer: 7200, denom: 1",
            "numer: 10800, denom: 1",
        ),
        (
            "HeLa incubated with 80–90% etoposide",
            "has_quantity",
            "numer: 4, denom: 5",
            "numer: 9, denom: 10",
        ),
        (
            "HeLa incubated with 45-60% etoposide",
            "has_quantity",
            "numer: 9, denom: 20",
            "numer: 3, denom: 5",
        ),
        (
            "the temperature was 30–37 °C",
            "has_quantity",
            "numer: 6063, denom: 20",
            "numer: 6203, denom: 20",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains(&format!("ontology:{relation}")),
            "{text}: {pretty}"
        );
        assert_eq!(
            debug.matches("urn:eigenius:units:le").count(),
            2,
            "{text}: {pretty}"
        );
        let (l, h) = (debug.find(low), debug.find(high));
        assert!(l.is_some() && h.is_some() && l < h, "{text}: {debug}");
    }
}

/// Slice 6d: a plain value is exact (slice 7's decision), so a bound is written out, before the value
/// or after it. `or more` and `or higher` are at least, `or less` and `or lower` at most; a postfix
/// bound takes its value on the left, through backward unit application, in every position a bound
/// takes.
#[test]
fn a_bound_can_follow_the_value() {
    let parser = Parser::build(layer());
    for (text, relation, value_first_expected, magnitude) in [
        (
            "HeLa incubated at 37 °C or higher",
            "prep_at_value",
            false,
            "numer: 6203, denom: 20",
        ),
        (
            "HeLa incubated with 10% or more",
            "prep_with_value",
            false,
            "numer: 1, denom: 10",
        ),
        (
            "HeLa incubated for 2 h or less",
            "prep_for_value",
            true,
            "numer: 7200, denom: 1",
        ),
        (
            "the temperature was 4 °C or lower",
            "has_quantity",
            true,
            "numer: 5543, denom: 20",
        ),
        (
            "the temperature was 37 °C or higher",
            "has_quantity",
            false,
            "numer: 6203, denom: 20",
        ),
        (
            "HeLa incubated with 10% or more etoposide",
            "has_quantity",
            false,
            "numer: 1, denom: 10",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains(&format!("ontology:{relation}")) && debug.contains("units:le"),
            "{text}: {pretty}"
        );
        assert!(debug.contains(magnitude), "{text}: {debug}");
        assert_eq!(
            value_first(&pretty, "le"),
            Some(value_first_expected),
            "{text}: {pretty}"
        );
    }
}

/// Slice 7a: a plain cardinal states the exact count of its scope set, `has_count(T, λx. V(x), n)` —
/// word or digit, subject or object, in scientific notation too; `0` reads, and `one` takes a
/// singular noun.
#[test]
fn a_plain_count_is_exact() {
    let parser = Parser::build(layer());
    for (text, count) in [
        ("two cells incubated", "numer: 2, denom: 1"),
        ("2 cells incubated", "numer: 2, denom: 1"),
        ("HeLa received two cells", "numer: 2, denom: 1"),
        ("HeLa received 1,000 cells", "numer: 1000, denom: 1"),
        ("HeLa received 2 × 10³ cells", "numer: 2000, denom: 1"),
        ("0 cells incubated", "numer: 0, denom: 1"),
        ("one cell incubated", "numer: 1, denom: 1"),
        ("1 cell incubated", "numer: 1, denom: 1"),
        ("ten cells incubated", "numer: 10, denom: 1"),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        assert!(
            debug.contains("ontology:has_count") && debug.contains("lexicon:Cell"),
            "{text}: {}",
            pretty_term(parsed[0].sem())
        );
        assert!(debug.contains(count), "{text}: {debug}");
    }
    assert!(readings(&parser, "two cell incubated").is_empty());
}

/// Slice 7b: a bounded count constrains the count, `∃q. C(q) ∧ has_count(T, …, q)`, through the markers
/// a measured value takes, before the number or after it. `fewer than` and `or fewer` bound counts
/// only; `more than one` takes a singular noun.
#[test]
fn a_bounded_count_constrains_the_count() {
    let parser = Parser::build(layer());
    for (text, constraint, value_first_expected, count) in [
        (
            "at least 1,000 cells incubated",
            "le",
            false,
            "numer: 1000, denom: 1",
        ),
        (
            "HeLa received at least 1,000 cells",
            "le",
            false,
            "numer: 1000, denom: 1",
        ),
        (
            "more than one cell incubated",
            "lt",
            false,
            "numer: 1, denom: 1",
        ),
        (
            "fewer than 5 cells incubated",
            "lt",
            true,
            "numer: 5, denom: 1",
        ),
        (
            "at most 5 cells incubated",
            "le",
            true,
            "numer: 5, denom: 1",
        ),
        (
            "HeLa received 5 or more cells",
            "le",
            false,
            "numer: 5, denom: 1",
        ),
        (
            "5 or fewer cells incubated",
            "le",
            true,
            "numer: 5, denom: 1",
        ),
        ("≥ 8 cells incubated", "le", false, "numer: 8, denom: 1"),
        (
            "HeLa received >17,000 cells",
            "lt",
            false,
            "numer: 17000, denom: 1",
        ),
        (
            "at least two cells incubated",
            "le",
            false,
            "numer: 2, denom: 1",
        ),
        (
            "two or more cells incubated",
            "le",
            false,
            "numer: 2, denom: 1",
        ),
        (
            "about 500 cells incubated",
            "approx",
            true,
            "numer: 500, denom: 1",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains("ontology:has_count")
                && debug.contains(&format!("units:{constraint}"))
                && debug.contains(count),
            "{text}: {pretty}"
        );
        assert_eq!(
            value_first(&pretty, constraint),
            Some(value_first_expected),
            "{text}: {pretty}"
        );
    }
}

/// Slice 7b: an en-dash pair with no unit is a count range, `∃q. lo ≤ q ≤ hi ∧ has_count(…, q)`.
#[test]
fn a_count_range_constrains_the_count() {
    let parser = Parser::build(layer());
    for text in ["4–7 cells incubated", "HeLa received 4–7 cells"] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        assert!(debug.contains("ontology:has_count"), "{text}: {debug}");
        assert_eq!(debug.matches("urn:eigenius:units:le").count(), 2, "{text}");
        let (lo, hi) = (
            debug.find("numer: 4, denom: 1"),
            debug.find("numer: 7, denom: 1"),
        );
        assert!(lo.is_some() && hi.is_some() && lo < hi, "{text}: {debug}");
    }
}

/// Slice 7: a bare number is not a measure phrase (D95, "Bare numerals and quantities share a carrier,
/// not a category"). `at 37` with no unit has no parse, and a bounded or plain count has its count
/// reading only — never cells given a quantity.
#[test]
fn a_bare_number_is_not_a_measure() {
    let parser = Parser::build(layer());
    assert!(readings(&parser, "HeLa incubated at 37").is_empty());
    for text in [
        "2 cells incubated",
        "at least 1,000 cells incubated",
        "4–7 cells incubated",
        "HeLa received 2 × 10³ cells",
    ] {
        let r = readings(&parser, text);
        assert_eq!(r.len(), 1, "{text}: {r:#?}");
        assert!(!r[0].contains("has_quantity"), "{text}: {r:#?}");
    }
}

/// Slice 7c: a proportion is `has_proportion(x, λy. V(y), q)` of the partitive's group — a definite
/// plural or a kind — with a percentage or `half` as its value, bounded or ranged as any value is, as a
/// subject, an object or a preposition's object. `none of`, `all of` and `most of` state it themselves.
#[test]
fn a_proportion_of_a_group() {
    let parser = Parser::build(layer());
    for (text, group, constraint, share) in [
        (
            "15% of the cells incubated",
            "ontology:the",
            None,
            "numer: 3, denom: 20",
        ),
        (
            "15% of cells incubated",
            "kind_of",
            None,
            "numer: 3, denom: 20",
        ),
        (
            "half of the cells incubated",
            "ontology:the",
            None,
            "numer: 1, denom: 2",
        ),
        (
            "HeLa received half of the cells",
            "ontology:the",
            None,
            "numer: 1, denom: 2",
        ),
        (
            "none of the cells incubated",
            "ontology:the",
            None,
            "numer: 0, denom: 1",
        ),
        (
            "all of the cells incubated",
            "ontology:the",
            None,
            "numer: 1, denom: 1",
        ),
        (
            "more than half of the cells incubated",
            "ontology:the",
            Some("lt"),
            "numer: 1, denom: 2",
        ),
        (
            "> half of the cells incubated",
            "ontology:the",
            Some("lt"),
            "numer: 1, denom: 2",
        ),
        (
            "most of the cells incubated",
            "ontology:the",
            Some("lt"),
            "numer: 1, denom: 2",
        ),
        (
            "HeLa incubated in more than half of the cells",
            "ontology:the",
            Some("lt"),
            "numer: 1, denom: 2",
        ),
        (
            "45–60% of the cells incubated",
            "ontology:the",
            Some("le"),
            "numer: 9, denom: 20",
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let debug = format!("{:?}", parsed[0].sem());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            debug.contains("ontology:has_proportion")
                && debug.contains(group)
                && debug.contains(share),
            "{text}: {pretty}"
        );
        match constraint {
            Some(c) => assert!(
                debug.contains(&format!("urn:eigenius:units:{c}\"")),
                "{text}: {pretty}"
            ),
            None => assert!(
                !debug.contains("urn:eigenius:units:lt\""),
                "{text}: {pretty}"
            ),
        }
    }
}
