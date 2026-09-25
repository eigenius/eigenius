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

//! Base conversion (D93 slice 6) against the chain's own units layer.

use eigenius_kernel::numeric::Rational;
use eigenius_kernel::units::convert::{ConvertError, Converted, Vocabulary};
use eigenius_kernel::units::{BaseDimension as B, Constant, Exponent, Unit};
use num_bigint::BigInt;
use num_traits::Pow;
use std::sync::OnceLock;

fn vocab() -> &'static Vocabulary {
    static V: OnceLock<Vocabulary> = OnceLock::new();
    V.get_or_init(|| {
        let ctx = eigenius_kernel::testing::bootstrap_context();
        Vocabulary::from_layer(ctx.head()).expect("the units layer is on the bootstrap chain")
    })
}

fn q(n: i64, d: i64) -> Rational {
    Rational::new(n.into(), d.into()).unwrap()
}

fn convert(value: Rational, stated: &str) -> Converted {
    vocab()
        .convert(&value, stated)
        .unwrap_or_else(|e| panic!("{stated:?}: {e}"))
}

fn assert_converts(value: Rational, stated: &str, coefficient: Rational, pi: i16, unit: &str) {
    let c = convert(value, stated);
    assert_eq!(
        c.magnitude.coefficient(),
        &coefficient,
        "{stated} coefficient"
    );
    assert_eq!(
        c.magnitude.constant_exponent(Constant::Pi),
        pi,
        "{stated} π"
    );
    assert_eq!(c.unit.to_canonical_string(), unit, "{stated} unit");
}

/// The magnitudes D93 states.
#[test]
fn d93s_worked_examples() {
    // A bare °C takes the point reading: 37 °C is 310.15 K.
    assert_converts(q(37, 1), "°C", q(6203, 20), 0, "K");
    // Four spellings, one value.
    assert_converts(q(5, 1), "mg/kg", q(1, 200_000), 0, "1");
    assert_converts(q(5, 1), "μg/g", q(1, 200_000), 0, "1");
    // The dalton is measured (CODATA 2022); 50 kDa converts through it.
    let da = Rational::new(
        BigInt::from(50_000i64) * BigInt::from(166_053_906_892i64),
        BigInt::from(10).pow(38u32),
    )
    .unwrap();
    assert_converts(q(50, 1), "kDa", da, 0, "kg");
    // An angle is dimensionless, and 37° is 37π/180.
    assert_converts(q(37, 1), "°", q(37, 180), 1, "1");
}

/// Every named unit, converted from `1`, gives back its own declared factor, power of π and
/// dimension — the units layer and the converter agree entry by entry.
#[test]
fn every_named_unit_round_trips() {
    for symbol in [
        "s", "m", "kg", "A", "K", "mol", "cd", "g", "rad", "sr", "Hz", "N", "Pa", "J", "W", "C",
        "V", "F", "Ω", "S", "Wb", "T", "H", "lm", "lx", "Bq", "Gy", "Sv", "kat", "min", "h", "d",
        "°", "′", "″", "ha", "L", "t", "Da", "eV", "au",
    ] {
        let u = vocab()
            .unit(symbol)
            .unwrap_or_else(|| panic!("{symbol} is declared"));
        let c = convert(q(1, 1), symbol);
        assert_eq!(c.magnitude.coefficient(), &u.factor, "{symbol}");
        assert_eq!(
            i64::from(c.magnitude.constant_exponent(Constant::Pi)),
            u.factor_pi,
            "{symbol}"
        );
        assert_eq!(c.unit, u.dimension, "{symbol}");
    }
    // °C is the one with an offset: 1 °C is 274.15 K.
    assert_converts(q(1, 1), "°C", q(5483, 20), 0, "K");
}

/// An exact symbol wins; otherwise the longest prefix whose remainder is a unit.
#[test]
fn symbols_resolve_exact_first_then_longest_prefix() {
    assert_converts(q(1, 1), "min", q(60, 1), 0, "s"); // the minute, not a milli-something
    assert_converts(q(1, 1), "cd", q(1, 1), 0, "cd"); // the candela, not a centi-day
    assert_converts(q(1, 1), "Pa", q(1, 1), 0, "s^-2·m^-1·kg"); // the pascal
    assert_converts(q(1, 1), "dam", q(10, 1), 0, "m"); // deca-metre: `da` beats `d`
    assert_converts(q(1, 1), "ms", q(1, 1000), 0, "s"); // millisecond
    assert_converts(q(1, 1), "mm", q(1, 1000), 0, "m");
    assert_converts(
        q(1, 1),
        "pH",
        q(1, 1_000_000_000_000),
        0,
        "s^-2·m^2·kg·A^-2",
    ); // picohenry
    assert_converts(q(1, 1), "kg", q(1, 1), 0, "kg"); // the kilogram itself
}

#[test]
fn one_value_has_one_result_however_the_unit_is_spelled() {
    for stated in ["m·s^-2", "m s^-2", "m/s^2", "s^-2·m"] {
        assert_converts(q(3, 1), stated, q(3, 1), 0, "s^-2·m");
    }
    assert_converts(q(50, 1), "1/s", q(50, 1), 0, "s^-1");
    assert_converts(q(1, 1), "m^(1/2)", q(1, 1), 0, "m^1/2");
}

/// The affine rule: the offset belongs to the POINT reading of a bare °C. Inside a compound the
/// reading is a difference, and °C converts as K.
#[test]
fn celsius_takes_its_offset_only_when_bare() {
    assert_converts(q(5, 1), "°C/min", q(1, 12), 0, "s^-1·K");
    // A power applies to the unit, not the value: 2 °C² is 2 K², with no offset either.
    assert_converts(q(2, 1), "°C^2", q(2, 1), 0, "K^2");
}

#[test]
fn kinds_come_back_beside_the_unit() {
    let c = convert(q(3, 1), "rad/s");
    assert_eq!(c.unit.to_canonical_string(), "s^-1");
    let plane = eigenius_kernel::ontology::well_known::iri("urn:eigenius:units:plane_angle");
    let solid = eigenius_kernel::ontology::well_known::iri("urn:eigenius:units:solid_angle");
    assert_eq!(c.kinds.numerator, vec![(plane.clone(), Exponent::ONE)]);
    assert!(c.kinds.denominator.is_empty());
    // The unit algebra, where both are 1, cannot say this; the metadata can.
    let c = convert(q(1, 1), "sr/rad");
    assert_eq!(c.unit, Unit::dimensionless());
    assert_eq!(c.kinds.numerator, vec![(solid, Exponent::ONE)]);
    assert_eq!(c.kinds.denominator, vec![(plane, Exponent::ONE)]);
    // A unit with no kind contributes none.
    assert_eq!(convert(q(1, 1), "m/s").kinds, Default::default());
}

#[test]
fn what_is_refused() {
    let refused = |stated: &str| {
        vocab()
            .convert(&q(1, 1), stated)
            .expect_err(&format!("{stated:?} should be refused"))
    };
    assert!(matches!(refused("kkg"), ConvertError::NotPrefixable { .. }));
    assert!(matches!(
        refused("mmin"),
        ConvertError::NotPrefixable { .. }
    ));
    assert!(matches!(refused("k°C"), ConvertError::NotPrefixable { .. }));
    assert!(matches!(refused("metre"), ConvertError::UnknownSymbol(_)));
    assert!(matches!(refused("µg"), ConvertError::UnknownSymbol(_))); // U+00B5, not U+03BC
    assert!(matches!(refused("m/s/s"), ConvertError::Malformed { .. }));
    assert!(matches!(refused(" m"), ConvertError::Malformed { .. }));
    assert!(matches!(refused("m^0"), ConvertError::Malformed { .. }));
    assert!(matches!(refused(""), ConvertError::Malformed { .. }));
    // √1000 is not rational, and π to a half power is not in the magnitude's form.
    assert!(matches!(refused("km^(1/2)"), ConvertError::Inexact(_)));
    assert!(matches!(refused("°^(1/2)"), ConvertError::Inexact(_)));
}

#[test]
fn a_rational_root_that_is_exact_is_admitted() {
    // (100 m²)^(1/2) — the hectare's factor 10000 has an exact square root, 100.
    assert_converts(q(1, 1), "ha^(1/2)", q(100, 1), 0, "m");
}

/// The dimension bookkeeping, checked against the algebra rather than a string.
#[test]
fn a_newton_is_a_kilogram_metre_per_second_squared() {
    let n = convert(q(1, 1), "N").unit;
    let expected = Unit::base(B::Mass)
        .mul(&Unit::base(B::Length))
        .unwrap()
        .div(&Unit::base(B::Time).pow(Exponent::integer(2)).unwrap())
        .unwrap();
    assert_eq!(n, expected);
}

// ── The ESL form and the Rust API's term, on the commit path ────────────────────────────────────

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::validation::{ValidationRule, Validator};
use std::sync::Arc;

const PROBES: &str = r#"
namespace core    = "urn:eigenius:core";
namespace eigentt = "urn:eigenius:eigentt";
namespace prov    = "urn:eigenius:prov";
namespace u       = "urn:eigenius:units";
namespace probe   = "urn:eigenius:probe";

axiom probe:is_speed       : u:Quantity(u"s^-1·m") -> Prop
axiom probe:is_temperature : u:Quantity(u"K") -> Prop
axiom probe:is_ratio       : u:Quantity(u"1") -> Prop
"#;

/// Compile `prop` as a claim beside [`PROBES`] and validate it on the bootstrap chain.
fn commit(prop: &str) -> Result<Vec<(ValidationRule, String)>, String> {
    let src = format!(
        "{PROBES}\nresource probe:claim : core:Resource {{\n    \
         prov:was_attributed_to = \"urn:eigenius:prov:agent:unattributed\";\n    \
         eigentt:proposition = type_expr( {prop} );\n}}\n"
    );
    let storage = LayerStorage::in_memory();
    let ctx = bootstrap_with_storage(storage.clone()).expect("bootstrap builds");
    let resources = eigenius_kernel::esl::compile(&src, ctx.head()).map_err(|errs| {
        errs.iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let mut b = LayerBuilder::new("probe", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add_resource");
    }
    let layer = Arc::new(b.build(storage));
    Ok(Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| (e.rule, e.message))
        .collect())
}

/// The prefix is the author's own — `u:` here, not `units:` — and the form still resolves.
#[test]
fn an_authored_quantity_commits_at_its_base_unit() {
    for prop in [
        r#"probe:is_speed(u:quantity(3, "m/s"))"#,
        r#"probe:is_speed(u:quantity(0.5r, "km/min"))"#,
        r#"probe:is_temperature(u:quantity(37, "°C"))"#,
        r#"probe:is_ratio(u:quantity(5, "mg/kg"))"#,
        r#"probe:is_ratio(u:quantity(37, "°"))"#,
    ] {
        let errs = commit(prop).unwrap_or_else(|e| panic!("{prop} should compile: {e}"));
        assert!(errs.is_empty(), "{prop} should commit: {errs:#?}");
    }
}

#[test]
fn an_authored_quantity_of_the_wrong_dimension_is_refused() {
    let errs = commit(r#"probe:is_speed(u:quantity(3, "m"))"#).expect("compiles");
    assert!(
        errs.iter()
            .any(|(rule, msg)| *rule == ValidationRule::TermIllTyped && msg.contains("Unit(m)")),
        "{errs:#?}"
    );
}

/// A stated unit the vocabulary cannot convert, or an inexact value, fails at COMPILE time, with
/// the reason — never as a silently wrong term.
#[test]
fn a_bad_stated_unit_or_value_fails_to_compile() {
    for (prop, needle) in [
        (
            r#"probe:is_speed(u:quantity(3, "metre/s"))"#,
            "not a unit symbol",
        ),
        (r#"probe:is_speed(u:quantity(3, "kkg"))"#, "does not prefix"),
        (
            r#"probe:is_speed(u:quantity(0.5, "m/s"))"#,
            "a float is not exact",
        ),
        (
            r#"probe:is_speed(u:quantity(3))"#,
            "takes a value and a stated unit",
        ),
    ] {
        let err = commit(prop).expect_err(&format!("{prop} should not compile"));
        assert!(err.contains(needle), "{prop}: {err}");
    }
}

/// The Rust API builds the same shape of term the ESL form elaborates to, and it type-checks at
/// its base unit.
#[test]
fn the_rust_api_term_type_checks() {
    use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
    use eigenius_kernel::nbe::env::Rho;
    use eigenius_kernel::nbe::readback::readback_val;
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let c = convert(q(3, 1), "m/s");
    let mut check = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(ctx.head()));
    let ty = check_infer(&mut check, &c.quantity_term()).expect("the quantity term type-checks");
    let printed = format!("{:?}", readback_val(0, &ty));
    assert!(printed.contains("Unit(s^-1·m)"), "{printed}");
}

#[test]
fn a_magnitude_round_trips_through_its_chain_pair() {
    use eigenius_kernel::units::Magnitude;
    let m = convert(q(37, 1), "°").magnitude;
    let (c, pi) = m.chain_pair();
    assert_eq!(Magnitude::from_chain_pair(c.clone(), pi).unwrap(), m);
    // A stored `pi` past 16 bits is refused, not truncated.
    assert!(Magnitude::from_chain_pair(c, 40_000).is_err());
}
