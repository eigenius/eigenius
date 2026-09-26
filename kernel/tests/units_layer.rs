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

//! The `units` bootstrap layer (D93): its vocabulary is what the SI says, and `units:Quantity`
//! types what it should.
//!
//! The dimension checks derive each unit INDEPENDENTLY from its SI definition — `J = N·m`,
//! `W = J/s` — through `units::Unit`'s algebra, rather than restating the strings the layer holds.
//! A typo in `units.esl` that is still canonical (`s^-2·m·kg` written for the pascal) passes Rule 21
//! and fails here.

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::numeric::Rational;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::units::{BaseDimension as B, Exponent, Unit};
use eigenius_kernel::validation::{ValidationRule, Validator};
use num_bigint::BigInt;
use num_traits::Pow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const UNITS: &str = "urn:eigenius:units:";

fn iri(local: &str) -> Iri {
    Iri::parse(&format!("{UNITS}{local}")).expect("well-formed IRI")
}

/// Every resource on the chain declared `is_a` the given units class, keyed by local name.
fn instances(class: &str) -> BTreeMap<String, Arc<Resource>> {
    let storage = LayerStorage::in_memory();
    let ctx = bootstrap_with_storage(storage).expect("bootstrap builds");
    let class = iri(class);
    let mut out = BTreeMap::new();
    let mut layer: Option<&Arc<Layer>> = Some(ctx.head());
    while let Some(l) = layer {
        for (id, r) in l.iter_resources() {
            if r.is_a().contains(&class) {
                let local = id
                    .as_str()
                    .strip_prefix(UNITS)
                    .expect("in the units namespace");
                out.insert(local.to_string(), r);
            }
        }
        layer = l.parent();
    }
    out
}

fn str_prop<'a>(r: &'a Resource, local: &str) -> &'a str {
    r.get(&iri(local))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("missing units:{local}"))
}

fn dimension(r: &Resource) -> Unit {
    Unit::parse_canonical(str_prop(r, "dimension")).expect("Rule 21 admitted it, so it parses")
}

fn factor(r: &Resource) -> Rational {
    Rational::parse_canonical(str_prop(r, "factor")).expect("Rule 21 admitted it, so it parses")
}

fn pow10(n: i32) -> Rational {
    let p = BigInt::from(10).pow(n.unsigned_abs());
    if n >= 0 {
        Rational::from_integer(p).unwrap()
    } else {
        Rational::new(BigInt::from(1), p).unwrap()
    }
}

#[test]
fn the_layer_holds_the_declared_vocabulary() {
    let units = instances("NamedUnit");
    // 7 base + the gram + 22 derived + D93's 9 non-SI + the 3 plane angles + standard gravity.
    assert_eq!(units.len(), 43, "{:?}", units.keys().collect::<Vec<_>>());
    assert_eq!(instances("Prefix").len(), 24);
}

/// Each class's symbols are distinct. ACROSS classes they collide on purpose — `m` is metre and
/// milli, `d` day and deci, `h` hour and hecto, `T` tesla and tera — and resolving that is the
/// span recogniser's job (D95), which is why the two are separate classes.
#[test]
fn symbols_are_unique_within_each_class() {
    for class in ["NamedUnit", "Prefix"] {
        let mut seen = BTreeSet::new();
        for (name, r) in instances(class) {
            let sym = str_prop(&r, "symbol").to_string();
            assert!(
                seen.insert(sym.clone()),
                "{class} symbol {sym:?} repeated at {name}"
            );
        }
    }
}

#[test]
fn every_dimension_matches_its_si_definition() {
    let b = Unit::base;
    let p = |u: &Unit, n: i16| u.pow(Exponent::integer(n)).unwrap();
    let (s, m, kg, a, k, mol, cd) = (
        b(B::Time),
        b(B::Length),
        b(B::Mass),
        b(B::Current),
        b(B::Temperature),
        b(B::Amount),
        b(B::LuminousIntensity),
    );
    // Angles are the group's identity (D93, "Kinds are metadata, not algebra"): what makes the
    // radian a PLANE angle is its `units:kind`, checked separately below.
    let rad = Unit::dimensionless();
    let sr = p(&rad, 2);
    let hz = s.recip().unwrap();
    let n = kg.mul(&m).unwrap().div(&p(&s, 2)).unwrap();
    let j = n.mul(&m).unwrap();
    let w = j.div(&s).unwrap();
    let c = a.mul(&s).unwrap();
    let v = w.div(&a).unwrap();
    let wb = v.mul(&s).unwrap();
    let lm = cd.mul(&sr).unwrap();

    let expected: BTreeMap<&str, Unit> = [
        ("second", s.clone()),
        ("metre", m.clone()),
        ("kilogram", kg.clone()),
        ("ampere", a.clone()),
        ("kelvin", k.clone()),
        ("mole", mol.clone()),
        ("candela", cd.clone()),
        ("gram", kg.clone()),
        ("radian", rad.clone()),
        ("steradian", sr.clone()),
        ("hertz", hz.clone()),
        ("newton", n.clone()),
        ("pascal", n.div(&p(&m, 2)).unwrap()),
        ("joule", j.clone()),
        ("watt", w.clone()),
        ("coulomb", c.clone()),
        ("volt", v.clone()),
        ("farad", c.div(&v).unwrap()),
        ("ohm", v.div(&a).unwrap()),
        ("siemens", a.div(&v).unwrap()),
        ("weber", wb.clone()),
        ("tesla", wb.div(&p(&m, 2)).unwrap()),
        ("henry", wb.div(&a).unwrap()),
        ("degree_celsius", k.clone()),
        ("lumen", lm.clone()),
        ("lux", lm.div(&p(&m, 2)).unwrap()),
        ("becquerel", hz.clone()),
        ("gray", j.div(&kg).unwrap()),
        ("sievert", j.div(&kg).unwrap()),
        ("katal", mol.div(&s).unwrap()),
        ("minute", s.clone()),
        ("hour", s.clone()),
        ("day", s.clone()),
        ("degree", rad.clone()),
        ("arcminute", rad.clone()),
        ("arcsecond", rad.clone()),
        ("hectare", p(&m, 2)),
        ("litre", p(&m, 3)),
        ("tonne", kg.clone()),
        ("dalton", kg.clone()),
        ("electronvolt", j.clone()),
        ("astronomical_unit", m.clone()),
        ("standard_gravity", m.div(&p(&s, 2)).unwrap()),
    ]
    .into_iter()
    .collect();

    let units = instances("NamedUnit");
    assert_eq!(
        units.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected.keys().copied().collect::<BTreeSet<_>>(),
    );
    for (name, r) in &units {
        assert_eq!(&dimension(r), &expected[name.as_str()], "units:{name}");
    }
}

/// The exact factors, including the two whose canonical forms nobody should have to write by hand.
#[test]
fn every_factor_is_the_exact_si_value() {
    let q = |n: i64, d: i64| Rational::new(n.into(), d.into()).unwrap();
    let units = instances("NamedUnit");
    let expected: BTreeMap<&str, (Rational, i64)> = [
        ("gram", (pow10(-3), 0)),
        ("minute", (q(60, 1), 0)),
        ("hour", (q(3600, 1), 0)),
        ("day", (q(86400, 1), 0)),
        ("degree", (q(1, 180), 1)),
        ("arcminute", (q(1, 10800), 1)),
        ("arcsecond", (q(1, 648000), 1)),
        ("hectare", (q(10000, 1), 0)),
        ("litre", (pow10(-3), 0)),
        ("tonne", (q(1000, 1), 0)),
        (
            "electronvolt",
            (
                Rational::new(BigInt::from(1602176634i64), BigInt::from(10).pow(28u32)).unwrap(),
                0,
            ),
        ),
        (
            "dalton",
            (
                Rational::new(BigInt::from(166053906892i64), BigInt::from(10).pow(38u32)).unwrap(),
                0,
            ),
        ),
        ("astronomical_unit", (q(149597870700, 1), 0)),
        ("standard_gravity", (q(196133, 20000), 0)),
    ]
    .into_iter()
    .collect();

    for (name, r) in &units {
        let pi: i64 = r
            .get(&iri("factor_pi"))
            .and_then(|v| v.as_integer())
            .expect("factor_pi is required");
        let (want, want_pi) = expected
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| (q(1, 1), 0)); // every other entry is an SI unit, factor 1
        assert_eq!(factor(r), want, "units:{name} factor");
        assert_eq!(pi, want_pi, "units:{name} factor_pi");
    }
}

#[test]
fn prefix_factors_are_exact_powers_of_ten() {
    let exps: BTreeMap<&str, i32> = [
        ("quecto", -30),
        ("ronto", -27),
        ("yocto", -24),
        ("zepto", -21),
        ("atto", -18),
        ("femto", -15),
        ("pico", -12),
        ("nano", -9),
        ("micro", -6),
        ("milli", -3),
        ("centi", -2),
        ("deci", -1),
        ("deca", 1),
        ("hecto", 2),
        ("kilo", 3),
        ("mega", 6),
        ("giga", 9),
        ("tera", 12),
        ("peta", 15),
        ("exa", 18),
        ("zetta", 21),
        ("yotta", 24),
        ("ronna", 27),
        ("quetta", 30),
    ]
    .into_iter()
    .collect();
    let prefixes = instances("Prefix");
    assert_eq!(
        prefixes.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        exps.keys().copied().collect::<BTreeSet<_>>()
    );
    for (name, r) in &prefixes {
        assert_eq!(factor(r), pow10(exps[name.as_str()]), "units:{name}");
    }
}

/// Mass prefixes attach to the gram; the kilogram takes none (the SI's own rule).
#[test]
fn prefixes_attach_to_the_gram_not_the_kilogram() {
    let units = instances("NamedUnit");
    let prefixable = |n: &str| {
        units[n]
            .get(&iri("prefixable"))
            .and_then(|v| v.as_boolean())
            .unwrap()
    };
    assert!(prefixable("gram"));
    assert!(!prefixable("kilogram"));
}

/// Two exceptions, each exactly once: the dalton is the only MEASURED factor, and the degree
/// Celsius the only AFFINE unit.
#[test]
fn only_the_dalton_is_measured_and_only_celsius_is_affine() {
    let units = instances("NamedUnit");
    let with = |local: &str| -> Vec<&str> {
        units
            .iter()
            .filter(|(_, r)| r.get(&iri(local)).is_some())
            .map(|(n, _)| n.as_str())
            .collect()
    };
    assert_eq!(with("factor_uncertainty"), vec!["dalton"]);
    assert_eq!(with("factor_source"), vec!["dalton"]);
    assert_eq!(with("offset"), vec!["degree_celsius"]);
    assert_eq!(
        str_prop(&units["degree_celsius"], "offset"),
        "5463/20",
        "273.15 exactly"
    );
}

/// Kinds are METADATA (D93, "Kinds are metadata, not algebra"): exactly the angular units carry
/// one, and every unit carrying one has the dimensionless value — the kind is what says which
/// dimensionless quantity it measures, and the value never does.
#[test]
fn kinds_annotate_exactly_the_angular_units() {
    let units = instances("NamedUnit");
    let kind_of = |r: &Resource| -> Option<String> {
        r.get(&iri("kind"))
            .and_then(|v| v.as_iri())
            .map(|k| k.as_str().strip_prefix(UNITS).unwrap().to_string())
    };
    let kinded: BTreeMap<&str, String> = units
        .iter()
        .filter_map(|(n, r)| kind_of(r).map(|k| (n.as_str(), k)))
        .collect();
    let expected: BTreeMap<&str, String> = [
        ("radian", "plane_angle"),
        ("degree", "plane_angle"),
        ("arcminute", "plane_angle"),
        ("arcsecond", "plane_angle"),
        ("steradian", "solid_angle"),
    ]
    .into_iter()
    .map(|(n, k)| (n, k.to_string()))
    .collect();
    assert_eq!(kinded, expected);
    for name in kinded.keys() {
        assert_eq!(
            dimension(&units[*name]),
            Unit::dimensionless(),
            "units:{name}"
        );
    }
    assert_eq!(instances("Kind").len(), 2);
}

// ── `units:Quantity` on the commit path ──────────────────────────────────────────────────────────

/// Compile ESL onto the real bootstrap chain — which now carries the units layer — and return the
/// validation errors.
fn validate_esl(source: &str) -> Vec<(ValidationRule, String)> {
    let storage = LayerStorage::in_memory();
    let ctx = bootstrap_with_storage(storage.clone()).expect("bootstrap builds");
    let resources = eigenius_kernel::esl::compile(source, ctx.head())
        .unwrap_or_else(|e| panic!("ESL must compile: {e:?}"));
    let mut b = LayerBuilder::new("probe", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add_resource");
    }
    let layer = Arc::new(b.build(storage));
    Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| (e.rule, e.message))
        .collect()
}

fn check_proposition(prop: &str) -> Vec<(ValidationRule, String)> {
    validate_esl(&format!(
        r#"
namespace core    = "urn:eigenius:core";
namespace eigentt = "urn:eigenius:eigentt";
namespace prov    = "urn:eigenius:prov";
namespace units   = "urn:eigenius:units";
namespace probe   = "urn:eigenius:probe";

axiom probe:is_dimensionless : units:Quantity(u"1") -> Prop
axiom probe:is_length : units:Quantity(u"m") -> Prop
axiom probe:is_speed  : units:Quantity(u"s^-1·m") -> Prop

resource probe:claim : core:Resource {{
    prov:was_attributed_to = "urn:eigenius:prov:agent:unattributed";
    eigentt:proposition = type_expr( {prop} );
}}"#
    ))
}

fn assert_commits(prop: &str) {
    let errs = check_proposition(prop);
    assert!(errs.is_empty(), "{prop}\nshould commit, got: {errs:#?}");
}

fn assert_mismatch(prop: &str, needle: &str) {
    let errs = check_proposition(prop);
    assert!(
        errs.iter()
            .any(|(rule, msg)| *rule == ValidationRule::TermIllTyped && msg.contains(needle)),
        "{prop}\nshould be ill-typed mentioning {needle:?}, got: {errs:#?}"
    );
}

/// `u` is a parameter, so the unit of a quantity term comes from the type it is checked against.
/// `mk_quantity(37/180, 1)` is 37° — `37π/180`, a dimensionless magnitude, since an angle is the
/// group's identity — only because `is_dimensionless` expects a dimensionless quantity.
#[test]
fn the_unit_comes_from_the_expected_type() {
    assert_commits(r#"probe:is_dimensionless(units:mk_quantity(r"37/180", 1))"#);
    assert_commits(r#"probe:is_speed((units:mk_quantity(r"3", 0) : units:Quantity(u"s^-1·m")))"#);
}

#[test]
fn a_quantity_of_the_wrong_unit_is_refused() {
    assert_mismatch(
        r#"probe:is_length((units:mk_quantity(r"37/180", 1) : units:Quantity(u"1")))"#,
        "Unit(1)",
    );
}

/// The coefficient is `core:rational`, and `core:integer` is not a subtype of it: an exact integer
/// coefficient is written `r"37"` or `37r`.
#[test]
fn the_coefficient_must_be_rational() {
    assert_mismatch(
        r#"probe:is_dimensionless(units:mk_quantity(37, 0))"#,
        "EigonPrimitive(Rational)",
    );
}

// ── Unit expressions (slice 5): `units:mul` / `units:pow` ────────────────────────────────────────

fn check_with_operators(prop: &str) -> Vec<(ValidationRule, String)> {
    validate_esl(&format!(
        r#"
namespace core    = "urn:eigenius:core";
namespace eigentt = "urn:eigenius:eigentt";
namespace prov    = "urn:eigenius:prov";
namespace units   = "urn:eigenius:units";
namespace probe   = "urn:eigenius:probe";

axiom probe:is_speed : units:Quantity(u"s^-1·m") -> Prop
axiom probe:ratio : forall (u : core:unit, v : core:unit) => units:Quantity(u) -> units:Quantity(v) -> units:Quantity(units:mul(u, units:pow(v, r"-1")))
axiom probe:needs_uv : forall (u : core:unit, v : core:unit) => units:Quantity(units:mul(u, v)) -> Prop

resource probe:claim : core:Resource {{
    prov:was_attributed_to = "urn:eigenius:prov:agent:unattributed";
    eigentt:proposition = type_expr( {prop} );
}}"#
    ))
}

fn assert_operators_commit(prop: &str) {
    let errs = check_with_operators(prop);
    assert!(errs.is_empty(), "{prop}\nshould commit, got: {errs:#?}");
}

fn assert_operators_refused(prop: &str, needle: &str) {
    let errs = check_with_operators(prop);
    assert!(
        errs.iter()
            .any(|(rule, msg)| *rule == ValidationRule::TermIllTyped && msg.contains(needle)),
        "{prop}\nshould be ill-typed mentioning {needle:?}, got: {errs:#?}"
    );
}

/// D52's ratio: a unit-generic function applied to literal units. Its result type,
/// `Quantity(mul(u, pow(v, -1)))`, becomes `Quantity(s^-1·m)` when `u := m, v := s` — so
/// substitution re-normalises and the closed product reduces to a literal.
#[test]
fn a_unit_generic_function_applied_to_literal_units_reduces() {
    assert_operators_commit(
        r#"probe:is_speed(probe:ratio(u"m", u"s", (units:mk_quantity(r"3", 0) : units:Quantity(u"m")), (units:mk_quantity(r"2", 0) : units:Quantity(u"s"))))"#,
    );
    // The same function with its arguments swapped gives s·m⁻¹, which is not a speed.
    assert_operators_refused(
        r#"probe:is_speed(probe:ratio(u"s", u"m", (units:mk_quantity(r"3", 0) : units:Quantity(u"s")), (units:mk_quantity(r"2", 0) : units:Quantity(u"m"))))"#,
        "Unit(s·m^-1)",
    );
}

/// Over unit VARIABLES: `q : Quantity(mul(v, u))` is accepted where `Quantity(mul(u, v))` is
/// expected — the open normalisation D93 requires and nanoda's `try_reduce_nat` does not do.
#[test]
fn an_open_product_commutes_on_the_commit_path() {
    assert_operators_commit(
        r#"forall (u : core:unit, v : core:unit, q : units:Quantity(units:mul(v, u))) => probe:needs_uv(u, v, q)"#,
    );
}

/// The negative that makes the positive mean something: `u·u` is not `u·v`.
#[test]
fn a_square_is_not_a_product_of_two_variables() {
    assert_operators_refused(
        r#"forall (u : core:unit, v : core:unit, q : units:Quantity(units:mul(u, u))) => probe:needs_uv(u, v, q)"#,
        "urn:eigenius:units:pow",
    );
}
