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

//! D95 slices 4 to 9 — measure phrases in the grammar, their consumers, bounds and ranges on them,
//! counts, proportions, a determiner's numeral and modifiers before a shared head, the positions a
//! measure phrase takes, and factors on count comparatives. The categories (`cat_mp`,
//! `cat_unit_forall`, `lexicon:Reading`), the `Difference` type, the unit-polymorphic application, the
//! seeding and the prepositions over a measured value (`closed-class.esl`) are in the bootstrap chain;
//! the fixture adds the content words around them, and a verb that takes only a difference in kelvin,
//! which slice 7's consumers will be shaped as. No DB, no reseed.

use std::sync::Arc;

use eigenius_kernel::dcg::verbalize::{unit_sense_names, verbalize, Vb};
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
// Slice 7d: counted conjuncts sharing a head (`five MSS and five MSI cell lines`) — two adjective
// modifiers, a noun modifier (a kind compound's left noun) and a plural head.
class lexicon:CellLine : lexicon:Entity { }
resource lexicon:cell_lines_n : lexicon:LexicalEntry {
    lexicon:form     = "cell lines";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:CellLine, lexicon:pl) );
    lexicon:sem      = lexicon:CellLine;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "cell lines";
}
axiom lexicon:msi_adj : lexicon:Entity -> Prop
resource lexicon:msi_a : lexicon:LexicalEntry {
    lexicon:form     = "MSI";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:msi_adj;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "msi";
}
axiom lexicon:mss_adj : lexicon:Entity -> Prop
resource lexicon:mss_a : lexicon:LexicalEntry {
    lexicon:form     = "MSS";
    lexicon:cat      = type_expr( lexicon:bwd(lexicon:m_all, lexicon:cat_s(lexicon:dcl, lexicon:adj), lexicon:cat_np(lexicon:Entity, lexicon:num_any)) );
    lexicon:sem      = lexicon:mss_adj;
    lexicon:sem_type = type_expr( lexicon:Entity -> Prop );
    lexicon:sense    = "mss";
}
class lexicon:Colon : lexicon:Entity { }
resource lexicon:colon_n : lexicon:LexicalEntry {
    lexicon:form     = "colon";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Colon, lexicon:sg) );
    lexicon:sem      = lexicon:Colon;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "colon";
}
// Slice 8: the event an offset is measured from.
class lexicon:Transduction : lexicon:Entity { }
resource lexicon:transduction_n : lexicon:LexicalEntry {
    lexicon:form     = "transduction";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Transduction, lexicon:mass) );
    lexicon:sem      = lexicon:Transduction;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "transduction";
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
        "HeLa incubated for nine days",
        "HeLa received an eight-day dose",
        "the two cells incubated",
        "five MSS and five MSI cell lines incubated",
        "HeLa received five MSS MSI and five MSI cell lines",
        "five MSS and five MSI cell lines of HeLa incubated",
        "15% of MSS, 22% of MSI and 30% of colon cells incubated",
        "HeLa incubated 72 h after transduction",
        "72 h after transduction, HeLa incubated",
        "HeLa incubated every 2–3 days",
        "HeLa received 5 mg of etoposide",
        "HeLa incubated by three weeks",
        "at least 1,000 cells per dose incubated",
        "HeLa received two cells per dose",
        "HeLa incubated 4 and 7 days after transduction",
        "HeLa received 0.56-fold fewer cells than HeLa",
        "HeLa received two-fold more cells compared with HeLa",
        "HeLa received a median 0.56-fold fewer cells compared to HeLa",
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
/// entries of slice 6a, the 19 symbol entries of 6b, the 8 postfix entries of 6d, `half` and the two
/// partitive `of`s of 7c, and slice 8's 21 offsets, 6 `later`, 6 `by`, 6 `every` and 6
/// pseudo-partitive `of`s take values.
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
    assert_eq!(readings.len(), 126, "{readings:?}");
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
    // A consumer takes the measure phrase on its right (a preposition) or on its left (a postfix
    // bound, an offset, `later`).
    let (mut forward, mut backward) = (0, 0);
    for (iri, r) in layer.iter_resources() {
        if !r.is_instance_of(&entry) {
            continue;
        }
        let consumer = entry_to_item(head, &r).unwrap_or_else(|e| panic!("{iri}: {e}"));
        if let Some(with_point) = apply(&consumer, &point, head, RightContext::Other) {
            let with_bound = apply(&consumer, &bound, head, RightContext::Other)
                .unwrap_or_else(|| panic!("{iri} takes 37 K and no bound in K"));
            assert_eq!(with_bound.cat(), with_point.cat(), "{iri}");
            forward += 1;
        }
        if let Some(with_point) = apply(&point, &consumer, head, RightContext::Other) {
            let with_bound = apply(&bound, &consumer, head, RightContext::Other)
                .unwrap_or_else(|| panic!("{iri} takes 37 K on its left and no bound in K"));
            assert_eq!(with_bound.cat(), with_point.cat(), "{iri}");
            backward += 1;
        }
    }
    assert_eq!((forward, backward), (82, 35));
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

/// Slice 7d, decisions 11 and 12: a number word is a numeral — its unit makes a quantity, and it
/// counts as a digit does — and a numeral joined by a hyphen to a unit name is a prenominal quantity.
/// A hyphen before a unit symbol is not read.
#[test]
fn a_number_word_is_a_numeral() {
    let parser = Parser::build(layer());
    for (text, relation, value) in [
        (
            "HeLa incubated for nine days",
            "prep_for_value",
            "numer: 777600, denom: 1",
        ),
        ("Nine cells incubated", "has_count", "numer: 9, denom: 1"),
        (
            "HeLa received an eight-day dose",
            "has_quantity",
            "numer: 691200, denom: 1",
        ),
        (
            "HeLa received a 8-day dose",
            "has_quantity",
            "numer: 691200, denom: 1",
        ),
    ] {
        let r = readings(&parser, text);
        assert_eq!(r.len(), 1, "{text}: {r:#?}");
        assert!(
            r[0].contains(&format!("ontology:{relation}\"")) && r[0].contains(value),
            "{text}: {r:#?}"
        );
    }
    assert!(readings(&parser, "HeLa received a 2-h dose").is_empty());
}

/// Slice 7d, decision 13: `the`, `these` and `those` take a numeral. The definite is
/// `the_count(A, q)`; a demonstrative's referent is a hole carrying the count, so the reading is open.
#[test]
fn a_determiner_takes_a_numeral() {
    let parser = Parser::build(layer());
    for (text, count) in [
        ("the two cells incubated", "numer: 2, denom: 1"),
        ("HeLa received the 4 cells", "numer: 4, denom: 1"),
        ("none of the two cells incubated", "numer: 2, denom: 1"),
    ] {
        let r = readings(&parser, text);
        assert_eq!(r.len(), 1, "{text}: {r:#?}");
        assert!(
            r[0].contains("ontology:the_count\"")
                && r[0].contains("lexicon:Cell")
                && r[0].contains(count),
            "{text}: {r:#?}"
        );
    }
    assert!(readings(&parser, "the two cell incubated").is_empty());
    let layer = layer();
    let text = "none of the two cells incubated";
    let names = unit_sense_names(text, &parser, &Identity, &layer);
    let said = verbalize(
        parser.parse(text, &Identity)[0].sem(),
        &Vb::surface(&names, &layer),
    );
    assert!(said.starts_with("0 of the 2 Cell"), "{said}");
    for text in ["these two cells incubated", "HeLa received those 2 cells"] {
        let (closed, open) = parser.parse_open(text, &Identity);
        assert!(
            closed.is_empty(),
            "{text}: {} closed readings",
            closed.len()
        );
        assert_eq!(open.len(), 1, "{text}");
        assert_eq!(open[0].holes.len(), 1, "{text}");
        let hole = &open[0].holes[0];
        assert!(pretty_term(&hole.ty).contains("Cell"), "{text}");
        assert!(
            format!("{:?}", hole.count).contains("numer: 2, denom: 1"),
            "{text}: {:?}",
            hole.count
        );
        assert!(open[0].skeleton().contains("×2"), "{}", open[0].skeleton());
    }
}

/// Slice 7d, decisions 14 and 15: counted conjuncts share their head noun. A determiner composes with
/// the modifiers after it, the compositions coordinate, and the coordination applies to the head —
/// one reading, each count over the head refined by its own modifiers: adjectives, a noun (a kind
/// compound's), stacked modifiers, a head refined itself, a comma list, bounded counts, the definite,
/// and a partitive over the bare plural.
#[test]
fn counted_conjuncts_share_their_head() {
    let parser = Parser::build(layer());
    for (text, conjuncts) in [
        (
            "five MSS and five MSI cell lines incubated",
            vec!["ΣG#0:CellLine. mss_adj(G#0)", "ΣG#0:CellLine. msi_adj(G#0)"],
        ),
        (
            "HeLa received 6 MSI and 5 MSS cell lines",
            vec!["ΣG#0:CellLine. msi_adj(G#0)", "ΣG#0:CellLine. mss_adj(G#0)"],
        ),
        (
            "HeLa received five colon and five MSI cells",
            vec![
                "ΣG#0:Cell. compound_kind(G#0, Colon)",
                "ΣG#0:Cell. msi_adj(G#0)",
            ],
        ),
        (
            "HeLa received five MSS MSI and five MSI cell lines",
            vec![
                "ΣG#0:CellLine. And(msi_adj(G#0), mss_adj(G#0))",
                "ΣG#0:CellLine. msi_adj(G#0)",
            ],
        ),
        (
            "five MSS and five MSI cell lines of HeLa incubated",
            vec![
                "ΣG#0:CellLine. And(prep_of(G#0, hela), mss_adj(G#0))",
                "ΣG#0:CellLine. And(prep_of(G#0, hela), msi_adj(G#0))",
            ],
        ),
        (
            "five MSS, five MSI and two colon cell lines incubated",
            vec![
                "ΣG#0:CellLine. mss_adj(G#0)",
                "ΣG#0:CellLine. msi_adj(G#0)",
                "ΣG#0:CellLine. compound_kind(G#0, Colon)",
            ],
        ),
        (
            "HeLa received at least five MSS and at least five MSI cell lines",
            vec!["ΣG#2:CellLine. mss_adj(G#2)", "ΣG#2:CellLine. msi_adj(G#2)"],
        ),
        (
            "the two MSS and the two MSI cell lines incubated",
            vec![
                "the_count(ΣG#0:CellLine. mss_adj(G#0)",
                "the_count(ΣG#0:CellLine. msi_adj(G#0)",
            ],
        ),
        (
            "15% of MSS, 22% of MSI and 30% of colon cells incubated",
            vec![
                "kind_of(ΣG#0:Cell. mss_adj(G#0))",
                "kind_of(ΣG#0:Cell. msi_adj(G#0))",
                "kind_of(ΣG#0:Cell. compound_kind(G#0, Colon))",
            ],
        ),
        (
            "HeLa incubated in 15% of MSS and 22% of MSI cells",
            vec![
                "kind_of(ΣG#0:Cell. mss_adj(G#0))",
                "kind_of(ΣG#0:Cell. msi_adj(G#0))",
            ],
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let pretty = pretty_term(parsed[0].sem());
        assert!(pretty.starts_with("And("), "{text}: {pretty}");
        for c in conjuncts {
            assert!(pretty.contains(c), "{text}: no `{c}` in {pretty}");
        }
    }
    // A single composed determiner does not apply to a head: `five MSS cell lines` reads once.
    assert_eq!(readings(&parser, "five MSS cell lines incubated").len(), 1);
}

/// Slice 8a, decisions 1–3: an offset is a measure phrase a temporal preposition takes on its left
/// (`72 h after transduction`), bounded as any value is; `post` is `after`; `later` is `after` an
/// understood time; `by` with a value is a deadline.
#[test]
fn an_offset_is_a_measure_phrase_before_a_preposition() {
    let parser = Parser::build(layer());
    for (text, relation, value) in [
        (
            "HeLa incubated 72 h after transduction",
            "prep_after_offset",
            "numer: 259200, denom: 1",
        ),
        (
            "HeLa incubated 6 h before transduction",
            "prep_before_offset",
            "numer: 21600, denom: 1",
        ),
        (
            "HeLa incubated 4 days post transduction",
            "prep_after_offset",
            "numer: 345600, denom: 1",
        ),
        (
            "HeLa incubated about 72 h after transduction",
            "prep_after_offset",
            "numer: 259200, denom: 1",
        ),
        (
            "HeLa incubated 2 days later",
            "prep_after_value",
            "numer: 172800, denom: 1",
        ),
        (
            "HeLa incubated by three weeks",
            "prep_by_value",
            "numer: 1814400, denom: 1",
        ),
    ] {
        let r = readings(&parser, text);
        assert_eq!(r.len(), 1, "{text}: {r:#?}");
        assert!(
            r[0].contains(&format!("ontology:{relation}\"")) && r[0].contains(value),
            "{text}: {r:#?}"
        );
    }
    let r = readings(&parser, "HeLa incubated about 72 h after transduction");
    assert!(r[0].contains("units:approx\""), "{r:#?}");
    assert!(r[0].contains("lexicon:Transduction"), "{r:#?}");
}

/// Slice 8a, decision 4: at the start of a sentence a VP adjunct modifies the subject, after a comma
/// or without one — the reading the adjunct gives after the verb.
#[test]
fn a_fronted_adjunct_modifies_the_subject() {
    let parser = Parser::build(layer());
    for (fronted, after_the_verb) in [
        ("After 72 h, HeLa incubated", "HeLa incubated after 72 h"),
        (
            "72 h after transduction, HeLa incubated",
            "HeLa incubated 72 h after transduction",
        ),
        ("At 37 °C, HeLa incubated", "HeLa incubated at 37 °C"),
        (
            "After transduction HeLa incubated",
            "HeLa incubated after transduction",
        ),
        (
            "Every 3 days, two cells incubated",
            "two cells incubated every 3 days",
        ),
    ] {
        let a = parser.parse(fronted, &Identity);
        let b = parser.parse(after_the_verb, &Identity);
        assert_eq!((a.len(), b.len()), (1, 1), "{fronted} / {after_the_verb}");
        assert_eq!(
            pretty_term(a[0].sem()),
            pretty_term(b[0].sem()),
            "{fronted} / {after_the_verb}"
        );
    }
    // Only at the start: an adjunct inside the sentence does not take the subject after it.
    assert!(readings(&parser, "HeLa received after 72 h two cells").is_empty());
}

/// Slice 8b, decision 5: `every N unit` is the period of a repeated procedure, `every_period(x, u,
/// q)`; a range is a constraint on it.
#[test]
fn every_n_unit_is_a_period() {
    let parser = Parser::build(layer());
    let r = readings(&parser, "HeLa incubated every 3 days");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(
        r[0].contains("ontology:every_period\"") && r[0].contains("numer: 259200, denom: 1"),
        "{r:#?}"
    );
    let r = readings(&parser, "HeLa incubated every 2–3 days");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(
        r[0].contains("ontology:every_period\"")
            && r[0].matches("urn:eigenius:units:le\"").count() == 2,
        "{r:#?}"
    );
}

/// Slice 8c, decision 6: a pseudo-partitive states an amount of the noun's stuff, as the prenominal
/// measure phrase does, in each dimension that measures one; a percentage stays the partitive's
/// proportion.
#[test]
fn a_pseudo_partitive_measures_the_noun() {
    let parser = Parser::build(layer());
    for (text, prenominal, value) in [
        (
            "HeLa received 5 mg of etoposide",
            "HeLa received 5 mg etoposide",
            "numer: 1, denom: 200000",
        ),
        (
            "HeLa received 10 μM of etoposide",
            "HeLa received 10 μM etoposide",
            "numer: 1, denom: 100",
        ),
        (
            "HeLa received 50 μl of etoposide",
            "HeLa received 50 μl etoposide",
            "numer: 1, denom: 20000000",
        ),
        (
            "HeLa received 2 h of transduction",
            "HeLa received 2 h transduction",
            "numer: 7200, denom: 1",
        ),
    ] {
        let a = readings(&parser, text);
        let b = readings(&parser, prenominal);
        assert_eq!(a.len(), 1, "{text}: {a:#?}");
        assert_eq!(a, b, "{text} / {prenominal}");
        assert!(
            a[0].contains("ontology:has_quantity\"") && a[0].contains(value),
            "{text}: {a:#?}"
        );
    }
    let r = readings(&parser, "15% of etoposide incubated");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(
        r[0].contains("ontology:has_proportion\"") && !r[0].contains("has_quantity"),
        "{r:#?}"
    );
}

/// Slice 8d, decision 8: `per` after a counted noun phrase distributes, `∀y:Y. Q(λx. And(prep_per(Y,
/// x, y), V(x)))`, as a subject and as an object.
#[test]
fn per_distributes_over_a_counted_noun_phrase() {
    let parser = Parser::build(layer());
    for text in [
        "two cells per dose incubated",
        "at least 1,000 cells per dose incubated",
        "HeLa received two cells per dose",
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        let pretty = pretty_term(parsed[0].sem());
        assert!(
            pretty.starts_with("ΠG#0:Dose.")
                && pretty.contains("has_count(Cell")
                && pretty.contains("prep_per(Dose, G#"),
            "{text}: {pretty}"
        );
    }
}

/// Slice 8e, decision 9: a list with its unit written once is a quantifier over the slot a measure
/// phrase fills — the consumer applied to each value, conjoined (`or`: disjoined).
#[test]
fn a_list_shares_its_unit() {
    let parser = Parser::build(layer());
    for (text, head, values) in [
        (
            "HeLa incubated 4 and 7 days after transduction",
            "And(",
            ["numer: 345600, denom: 1", "numer: 604800, denom: 1"],
        ),
        (
            "HeLa incubated for 5 or 10 h",
            "Or(",
            ["numer: 18000, denom: 1", "numer: 36000, denom: 1"],
        ),
        (
            "Four and seven days after transduction, HeLa incubated",
            "And(",
            ["numer: 345600, denom: 1", "numer: 604800, denom: 1"],
        ),
    ] {
        let parsed = parser.parse(text, &Identity);
        assert_eq!(parsed.len(), 1, "{text}: {} readings", parsed.len());
        assert!(pretty_term(parsed[0].sem()).starts_with(head), "{text}");
        let debug = format!("{:?}", parsed[0].sem());
        assert!(values.iter().all(|v| debug.contains(v)), "{text}: {debug}");
    }
}

/// Slice 9, decisions 1 and 2: a factor on a count comparative states the factor as written,
/// `fold_lower(N, card(T, x), card(T, y))`. A percentage or a count before `fewer` is a difference,
/// not a factor, and is not read as one.
#[test]
fn a_factor_on_a_count_comparative() {
    let layer = layer();
    let parser = Parser::build(Arc::clone(&layer));
    let names = std::collections::BTreeMap::new();
    for (text, rel, value, words) in [
        (
            "HeLa received 0.56-fold fewer cells than HeLa",
            "ontology:fold_lower\"",
            "numer: 14, denom: 25",
            "hela has 14/25-fold fewer Cell than hela",
        ),
        (
            "HeLa received two-fold more cells than HeLa",
            "ontology:fold_higher\"",
            "numer: 2, denom: 1",
            "hela has 2-fold more Cell than hela",
        ),
    ] {
        let r = readings(&parser, text);
        assert_eq!(r.len(), 1, "{text}: {r:#?}");
        assert!(r[0].contains(rel) && r[0].contains(value), "{text}: {r:#?}");
        let parsed = parser.parse(text, &Identity);
        let surface = verbalize(parsed[0].sem(), &Vb::surface(&names, &layer));
        assert_eq!(surface, words, "{text}");
    }
    for text in [
        "HeLa received 10% fewer cells than HeLa",
        "HeLa received 3 fewer cells than HeLa",
    ] {
        let r = readings(&parser, text);
        assert!(r.iter().all(|r| !r.contains("fold_")), "{text}: {r:#?}");
    }
}

/// Slice 9, decision 4: `compared to` and `compared with` mark a comparison's standard as `than`
/// does, with and without a factor.
#[test]
fn compared_to_marks_the_standard() {
    let parser = Parser::build(layer());
    for (than, compared) in [
        (
            "HeLa received 0.56-fold fewer cells than HeLa",
            "HeLa received 0.56-fold fewer cells compared to HeLa",
        ),
        (
            "HeLa received fewer cells than HeLa",
            "HeLa received fewer cells compared with HeLa",
        ),
    ] {
        let a = readings(&parser, than);
        assert_eq!(a.len(), 1, "{than}: {a:#?}");
        assert_eq!(a, readings(&parser, compared), "{compared}");
    }
}

/// Slice 9, decision 3: `a median` is a statistic over a group's members, which the factor
/// comparative applies to both counts: `fold_lower(N, median_over(λm. card(T, m), x),
/// median_over(λm. card(T, m), y))`.
#[test]
fn a_median_summarises_both_counts() {
    let layer = layer();
    let parser = Parser::build(Arc::clone(&layer));
    let names = std::collections::BTreeMap::new();
    let text = "HeLa received a median 0.56-fold fewer cells compared to HeLa";
    let parsed = parser.parse(text, &Identity);
    assert_eq!(parsed.len(), 1, "{} readings", parsed.len());
    let pretty = pretty_term(parsed[0].sem());
    assert!(
        pretty.starts_with("fold_lower(")
            && pretty.matches("median_over(λ").count() == 2
            && pretty.matches("card(Cell, ").count() == 2,
        "{pretty}"
    );
    let surface = verbalize(parsed[0].sem(), &Vb::surface(&names, &layer));
    assert_eq!(surface, "hela has a median 14/25-fold fewer Cell than hela");
    // A statistic needs a factor: `a median fewer cells` is not English and has no reading.
    assert!(parser
        .parse("HeLa received a median fewer cells than HeLa", &Identity)
        .is_empty());
}
