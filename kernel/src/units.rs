//! Units of measure — the `Unit` primitive's algebra (D93).
//!
//! An exponent vector and a coefficient, and no expressions anywhere. That is what keeps symbolic
//! algebra out of the kernel: there is nothing to reorder, so equality is structural.
//!
//! | | structure | kernel operations |
//! |---|---|---|
//! | unit exponents | ℚ⁷ over the seven base dimensions | add, subtract, scalar-multiply, compare |
//! | magnitude | ℚ × ℤ^C — a rational coefficient and integer powers of declared constants | **compare only** |
//!
//! **A unit is an element of a group, and nothing else.** Quantity KINDS — plane angle, solid
//! angle — are not part of it. An earlier version carried a kind vector with a rule dropping it once
//! the dimension vector was non-zero; that made `m·rad = m` while `rad ≠ 1`, which no group admits
//! (cancel `m`), and the product was not associative: `(m·rad)·m⁻¹` gave `1` and `rad·(m·m⁻¹)`
//! gave `rad`. Kinds are now metadata on the units layer's vocabulary, outside equality (D93,
//! "Kinds are metadata, not algebra").
//!
//! The exponents are [`Exponent`], a fixed-width rational: 16 bits of numerator and denominator
//! allows `m^32767`, and D94's arbitrary-precision [`crate::numeric::Rational`] is not needed here
//! (D94, "Two chain primitives, not three"). It carries the magnitude's *coefficient* instead.
//!
//! The SI *content* — 22 named derived units, 24 prefixes, the non-SI accepted list — is not here.
//! It lives in chain ontology, where it is authored, reviewed and replaceable without touching the
//! checker. Those are abbreviations that normalise away, not primitives.

use crate::numeric::Rational;
use std::collections::BTreeMap;
use std::fmt;

/// The seven SI base dimensions. `mol` stays one (D93, "`mol` stays a base dimension").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BaseDimension {
    /// second
    Time,
    /// metre
    Length,
    /// kilogram
    Mass,
    /// ampere
    Current,
    /// kelvin
    Temperature,
    /// mole
    Amount,
    /// candela
    LuminousIntensity,
}

impl BaseDimension {
    /// All seven, in the canonical order exponent vectors are indexed by.
    pub const ALL: [BaseDimension; 7] = [
        BaseDimension::Time,
        BaseDimension::Length,
        BaseDimension::Mass,
        BaseDimension::Current,
        BaseDimension::Temperature,
        BaseDimension::Amount,
        BaseDimension::LuminousIntensity,
    ];

    /// The SI symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            BaseDimension::Time => "s",
            BaseDimension::Length => "m",
            BaseDimension::Mass => "kg",
            BaseDimension::Current => "A",
            BaseDimension::Temperature => "K",
            BaseDimension::Amount => "mol",
            BaseDimension::LuminousIntensity => "cd",
        }
    }

    /// The inverse of [`BaseDimension::symbol`]. Exact — no aliases, no case folding.
    pub fn from_symbol(sym: &str) -> Option<BaseDimension> {
        BaseDimension::ALL.into_iter().find(|d| d.symbol() == sym)
    }
}

/// A declared constant a magnitude may carry integer powers of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Constant {
    /// π. Admits the transcendental unit category — angles, the parsec, atomic units — which a
    /// strict rational magnitude would have excluded by consequence rather than by choice.
    Pi,
}

/// Why a unit operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitError {
    /// An exponent went outside the fixed width. Refused rather than wrapped: a silently wrapped
    /// exponent is a different unit.
    ExponentOverflow,
    /// A zero denominator in an exponent.
    ZeroDenominator,
    /// A string was not the canonical form of a unit or magnitude. Carries the offending input.
    Malformed(String),
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::ExponentOverflow => write!(f, "unit exponent outside the 16-bit range"),
            UnitError::ZeroDenominator => write!(f, "unit exponent has a zero denominator"),
            UnitError::Malformed(input) => write!(f, "not a canonical unit form: {input:?}"),
        }
    }
}

impl std::error::Error for UnitError {}

/// A fixed-width rational exponent, always canonical: reduced, with a positive denominator.
///
/// Sixteen bits each allows `m^32767`, which is past anything a unit needs; the width is what keeps
/// these out of arbitrary precision (D94).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Exponent {
    numer: i16,
    denom: u16,
}

impl Exponent {
    /// Zero — the exponent of a base that does not appear.
    pub const ZERO: Exponent = Exponent { numer: 0, denom: 1 };
    /// One.
    pub const ONE: Exponent = Exponent { numer: 1, denom: 1 };

    /// Builds an exponent, reducing it.
    pub fn new(numer: i16, denom: i16) -> Result<Exponent, UnitError> {
        if denom == 0 {
            return Err(UnitError::ZeroDenominator);
        }
        let (mut n, mut d) = (i32::from(numer), i32::from(denom));
        if d < 0 {
            n = -n;
            d = -d;
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs());
        if g != 0 {
            n /= g as i32;
            d /= g as i32;
        }
        Ok(Exponent {
            numer: i16::try_from(n).map_err(|_| UnitError::ExponentOverflow)?,
            denom: u16::try_from(d).map_err(|_| UnitError::ExponentOverflow)?,
        })
    }

    /// Builds an integer exponent.
    /// The exponent a canonical rational denotes, refused when a part leaves the fixed width.
    ///
    /// A canonical [`Rational`] is already reduced with a positive denominator, which is this
    /// type's invariant, so the parts are taken as they are.
    pub fn from_rational(r: &Rational) -> Result<Exponent, UnitError> {
        use num_traits::ToPrimitive;
        let numer = r.numer().to_i16().ok_or(UnitError::ExponentOverflow)?;
        let denom = r.denom().to_u16().ok_or(UnitError::ExponentOverflow)?;
        Ok(Exponent { numer, denom })
    }

    /// This exponent as a canonical rational.
    pub fn to_rational(self) -> Rational {
        // Both parts fit 16 bits and the pair is reduced with a positive denominator, so this is
        // always an admissible rational, far inside D94's bound.
        Rational::new(self.numer.into(), self.denom.into())
            .expect("an Exponent is a reduced, bounded rational")
    }

    pub fn integer(n: i16) -> Exponent {
        Exponent { numer: n, denom: 1 }
    }

    /// Whether this is zero.
    pub fn is_zero(self) -> bool {
        self.numer == 0
    }

    /// Adds two exponents, which is what multiplying two units does to a base.
    ///
    /// Named like `i32::checked_add`: the std trait cannot be implemented because this REFUSES on
    /// overflow rather than wrapping, and a wrapped exponent is a different unit.
    pub fn checked_add(self, other: Exponent) -> Result<Exponent, UnitError> {
        let n = i32::from(self.numer) * i32::from(other.denom)
            + i32::from(other.numer) * i32::from(self.denom);
        let d = i32::from(self.denom) * i32::from(other.denom);
        Self::from_i32(n, d)
    }

    /// Negates, which is what inverting a unit does to a base.
    pub fn checked_neg(self) -> Result<Exponent, UnitError> {
        Ok(Exponent {
            numer: self
                .numer
                .checked_neg()
                .ok_or(UnitError::ExponentOverflow)?,
            denom: self.denom,
        })
    }

    /// Multiplies by another exponent, which is what raising a unit to a power does.
    pub fn checked_mul(self, other: Exponent) -> Result<Exponent, UnitError> {
        let n = i32::from(self.numer) * i32::from(other.numer);
        let d = i32::from(self.denom) * i32::from(other.denom);
        Self::from_i32(n, d)
    }

    fn from_i32(mut n: i32, mut d: i32) -> Result<Exponent, UnitError> {
        if d == 0 {
            return Err(UnitError::ZeroDenominator);
        }
        if d < 0 {
            n = -n;
            d = -d;
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs());
        if g != 0 {
            n /= g as i32;
            d /= g as i32;
        }
        Ok(Exponent {
            numer: i16::try_from(n).map_err(|_| UnitError::ExponentOverflow)?,
            denom: u16::try_from(d).map_err(|_| UnitError::ExponentOverflow)?,
        })
    }
}

impl fmt::Display for Exponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denom == 1 {
            write!(f, "{}", self.numer)
        } else {
            write!(f, "{}/{}", self.numer, self.denom)
        }
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// A unit: a rational exponent vector over the seven base dimensions.
///
/// An element of the free Abelian group over the base dimensions with rational exponents —
/// Kennedy's group, with ℚ in place of ℤ. Multiplication adds exponents, so it is associative and
/// commutative, every unit has an inverse, and the dimensionless unit is the identity. Canonical by
/// construction: the exponents are a fixed-order array of reduced [`Exponent`]s, so structural
/// equality is value equality.
///
/// `rad`, `sr`, `°` and `1` are all this group's identity, as in the SI, so `s = rθ` gives metres.
/// What distinguishes a plane angle from a solid angle is a KIND, which the units layer records as
/// metadata on the named unit and which never enters this value.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Unit {
    dimension: [Exponent; 7],
}

impl Unit {
    /// The dimensionless unit — the group's identity.
    pub fn dimensionless() -> Unit {
        Unit {
            dimension: [Exponent::ZERO; 7],
        }
    }

    /// A base unit raised to the first power.
    pub fn base(d: BaseDimension) -> Unit {
        let mut dimension = [Exponent::ZERO; 7];
        dimension[d as usize] = Exponent::ONE;
        Unit { dimension }
    }

    /// The exponent of one base dimension.
    pub fn exponent_of(&self, d: BaseDimension) -> Exponent {
        self.dimension[d as usize]
    }

    /// Whether every base exponent is zero.
    pub fn is_dimensionless(&self) -> bool {
        self.dimension.iter().all(|e| e.is_zero())
    }

    /// Multiplies two units: add the exponents.
    pub fn mul(&self, other: &Unit) -> Result<Unit, UnitError> {
        let mut dimension = [Exponent::ZERO; 7];
        for (out, (a, b)) in dimension
            .iter_mut()
            .zip(self.dimension.iter().zip(other.dimension.iter()))
        {
            *out = a.checked_add(*b)?;
        }
        Ok(Unit { dimension })
    }

    /// The inverse: negate every exponent.
    pub fn recip(&self) -> Result<Unit, UnitError> {
        let mut dimension = [Exponent::ZERO; 7];
        for (out, e) in dimension.iter_mut().zip(self.dimension.iter()) {
            *out = e.checked_neg()?;
        }
        Ok(Unit { dimension })
    }

    /// Divides: multiply by the reciprocal.
    pub fn div(&self, other: &Unit) -> Result<Unit, UnitError> {
        self.mul(&other.recip()?)
    }

    /// Raises to a rational power: multiply every exponent.
    pub fn pow(&self, p: Exponent) -> Result<Unit, UnitError> {
        let mut dimension = [Exponent::ZERO; 7];
        for (out, e) in dimension.iter_mut().zip(self.dimension.iter()) {
            *out = e.checked_mul(p)?;
        }
        Ok(Unit { dimension })
    }

    /// Renders the canonical form: base symbols in [`BaseDimension::ALL`] order joined by `·`, with
    /// `1` for the dimensionless unit and no explicit `^1`.
    pub fn to_canonical_string(&self) -> String {
        self.to_string()
    }

    /// Parses the canonical form, refusing every other spelling of the same value.
    ///
    /// `m·s^-1·m` is REFUSED even though it denotes `s^-1·m^2`, and so are `m^1`, `m^0` and a
    /// repeated symbol. The reason is the one
    /// [`crate::numeric::Rational::parse_canonical`] gives: the content hash runs over the
    /// serialised resource, so two spellings of one value would hash differently, and a parser that
    /// accepted both would make that unobservable.
    ///
    /// The closing round-trip check is what enforces it. Rather than enumerating the ways an input
    /// can be non-canonical, this builds the value and then demands the input already be what that
    /// value prints as — so every future change to canonical form tightens the parser for free.
    pub fn parse_canonical(s: &str) -> Result<Unit, UnitError> {
        let bad = || UnitError::Malformed(s.to_string());
        if s == "1" {
            return Ok(Unit::dimensionless());
        }
        if s.is_empty() {
            return Err(bad());
        }
        let mut acc = Unit::dimensionless();
        for part in s.split('\u{b7}') {
            let (sym, exp) = match part.split_once('^') {
                Some((sym, e)) => (sym, parse_exponent(e).ok_or_else(bad)?),
                None => (part, Exponent::ONE),
            };
            let Some(d) = BaseDimension::from_symbol(sym) else {
                return Err(bad());
            };
            let factor = Unit::base(d).pow(exp)?;
            acc = acc.mul(&factor)?;
        }
        if acc.to_canonical_string() != s {
            return Err(bad());
        }
        Ok(acc)
    }
}

/// Parses one exponent — `-?uint` or `-?uint/uint`, canonical only.
fn parse_exponent(s: &str) -> Option<Exponent> {
    let (num_str, den_str) = match s.split_once('/') {
        Some((n, d)) => (n, Some(d)),
        None => (s, None),
    };
    let (negative, digits) = match num_str.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, num_str),
    };
    // `^0` has no spelling: a zero exponent is DROPPED by canonicalisation.
    if !is_canonical_uint(digits) || digits == "0" {
        return None;
    }
    let numer: i16 = digits.parse().ok()?;
    let numer = if negative { -numer } else { numer };
    let denom = match den_str {
        None => 1,
        Some(d) => {
            // `/1` is a second spelling of the integer form.
            if !is_canonical_uint(d) || d == "0" || d == "1" {
                return None;
            }
            d.parse().ok()?
        }
    };
    Exponent::new(numer, denom).ok()
}

/// `0`, or a digit string with no leading zero. No sign, no whitespace, no `+`.
fn is_canonical_uint(s: &str) -> bool {
    match s.as_bytes() {
        [] => false,
        [b'0'] => true,
        [b'0', ..] => false,
        bytes => bytes.iter().all(|b| b.is_ascii_digit()),
    }
}

/// The canonical form, not the exponent array. Type-mismatch errors print both sides with `{:?}`,
/// and seven `Exponent { numer, denom }` structs per side made `s ≠ m` unreadable.
impl fmt::Debug for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unit({self})")
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        for d in BaseDimension::ALL {
            let e = self.exponent_of(d);
            if !e.is_zero() {
                parts.push(if e == Exponent::ONE {
                    d.symbol().to_string()
                } else {
                    format!("{}^{}", d.symbol(), e)
                });
            }
        }
        if parts.is_empty() {
            f.write_str("1")
        } else {
            f.write_str(&parts.join("·"))
        }
    }
}

/// A magnitude: `q × Π cᵢ^{eᵢ}`, a reduced rational coefficient and integer powers of constants.
///
/// A canonical datum, not a formula. `37π/180` and `π·37/180` are both `(37/180, [π ↦ 1])` and
/// compare syntactically — there is nothing to reorder, which is the point: making `37 × π` and
/// `π × 37` compare equal *as expressions* would need AC-normalisation, and that is symbolic
/// algebra in the trusted surface.
///
/// **A multiplicative group, deliberately not a ring.** `37π/180 + 1/2` escapes the form entirely;
/// magnitudes are not closed under addition and do not need to be, because the kernel's only
/// magnitude operation is equality. Conversion multiplies, and happens at ingest, outside the TCB.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Magnitude {
    coefficient: Rational,
    constants: BTreeMap<Constant, i16>,
}

impl Magnitude {
    /// A magnitude with no constant factors.
    pub fn rational(coefficient: Rational) -> Magnitude {
        Magnitude {
            coefficient,
            constants: BTreeMap::new(),
        }
    }

    /// A magnitude carrying one constant to a power. A zero power is dropped.
    pub fn with_constant(mut self, c: Constant, power: i16) -> Magnitude {
        if power == 0 {
            self.constants.remove(&c);
        } else {
            self.constants.insert(c, power);
        }
        self
    }

    /// The rational coefficient.
    pub fn coefficient(&self) -> &Rational {
        &self.coefficient
    }

    /// The exponent of one constant, zero when absent.
    pub fn constant_exponent(&self, c: Constant) -> i16 {
        self.constants.get(&c).copied().unwrap_or(0)
    }
}

impl fmt::Display for Magnitude {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.coefficient)?;
        for (c, e) in &self.constants {
            let name = match c {
                Constant::Pi => "π",
            };
            if *e == 1 {
                write!(f, "·{name}")?;
            } else {
                write!(f, "·{name}^{e}")?;
            }
        }
        Ok(())
    }
}

/// Serialises as the canonical STRING, never as a structure.
///
/// The same reasoning as [`crate::numeric::Rational`]'s: making this the only serde representation
/// means every path through serde is canonical by construction rather than by each call site
/// remembering. A structural form would also put the exponent vector's interior into the serialised
/// resource, where `core:mentions` walks it and the content hash covers it (D93).
impl serde::Serialize for Unit {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> serde::Deserialize<'de> for Unit {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(de)?;
        Unit::parse_canonical(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Unit {
        Unit::base(BaseDimension::Length)
    }
    fn s() -> Unit {
        Unit::base(BaseDimension::Time)
    }

    // ── The group laws ─────────────────────────────────────────────────
    //
    // An earlier canonical form dropped kinds once the dimension was non-zero, and failed the first
    // of these: `(m·rad)·m⁻¹` gave `1` and `rad·(m·m⁻¹)` gave `rad`. Every open-expression
    // normaliser built on this type assumes these laws, so they are pinned here.

    fn samples() -> Vec<Unit> {
        let kg = Unit::base(BaseDimension::Mass);
        vec![
            Unit::dimensionless(),
            m(),
            s(),
            m().div(&s()).unwrap(),
            kg.mul(&m())
                .unwrap()
                .div(&s().pow(Exponent::integer(2)).unwrap())
                .unwrap(),
            m().pow(Exponent::new(2, 3).unwrap()).unwrap(),
        ]
    }

    #[test]
    fn the_product_is_associative() {
        for a in samples() {
            for b in samples() {
                for c in samples() {
                    let left = a.mul(&b).unwrap().mul(&c).unwrap();
                    let right = a.mul(&b.mul(&c).unwrap()).unwrap();
                    assert_eq!(left, right, "({a}·{b})·{c} ≠ {a}·({b}·{c})");
                }
            }
        }
    }

    #[test]
    fn the_product_is_commutative() {
        for a in samples() {
            for b in samples() {
                assert_eq!(a.mul(&b).unwrap(), b.mul(&a).unwrap(), "{a}·{b}");
            }
        }
    }

    #[test]
    fn every_unit_has_an_inverse_and_one_is_the_identity() {
        for a in samples() {
            assert_eq!(a.mul(&a.recip().unwrap()).unwrap(), Unit::dimensionless());
            assert_eq!(a.mul(&Unit::dimensionless()).unwrap(), a);
        }
    }

    /// Arc length, `s = rθ`: with angle the identity, a metre times an angle is a metre, and
    /// a ratio of lengths is the dimensionless unit — the same value an angle is.
    #[test]
    fn a_ratio_of_lengths_is_the_identity() {
        assert_eq!(m().div(&m()).unwrap(), Unit::dimensionless());
    }

    // ── The exponent vector ────────────────────────────────────────────

    #[test]
    fn multiplying_adds_exponents_and_dividing_subtracts_them() {
        let speed = m().div(&s()).unwrap();
        assert_eq!(speed.exponent_of(BaseDimension::Length), Exponent::ONE);
        assert_eq!(
            speed.exponent_of(BaseDimension::Time),
            Exponent::integer(-1)
        );
        let accel = speed.div(&s()).unwrap();
        assert_eq!(
            accel.exponent_of(BaseDimension::Time),
            Exponent::integer(-2)
        );
    }

    #[test]
    fn kennedy_normalisation_reduces_a_repeated_base() {
        // `m · s⁻¹ · m → m² s⁻¹`: there is no list to reorder, because a unit IS the vector.
        let u = m().mul(&s().recip().unwrap()).unwrap().mul(&m()).unwrap();
        assert_eq!(u.exponent_of(BaseDimension::Length), Exponent::integer(2));
        assert_eq!(u.exponent_of(BaseDimension::Time), Exponent::integer(-1));
        assert_eq!(u.to_string(), "s^-1·m^2");
    }

    #[test]
    fn rational_exponents_compose() {
        // `m^(1/2) · m^(1/2) = m` — the exponent arithmetic D94 names as this algebra's dependency.
        let half = Exponent::new(1, 2).unwrap();
        let root = m().pow(half).unwrap();
        assert_eq!(root.mul(&root).unwrap(), m());
    }

    #[test]
    fn exponents_are_reduced_and_sign_normalised() {
        assert_eq!(Exponent::new(2, 4).unwrap(), Exponent::new(1, 2).unwrap());
        assert_eq!(Exponent::new(1, -2).unwrap(), Exponent::new(-1, 2).unwrap());
        assert_eq!(Exponent::new(0, 5).unwrap(), Exponent::ZERO);
    }

    #[test]
    fn a_zero_denominator_exponent_is_refused() {
        assert_eq!(Exponent::new(1, 0), Err(UnitError::ZeroDenominator));
    }

    #[test]
    fn exponent_overflow_is_refused_not_wrapped() {
        // A silently wrapped exponent is a different unit.
        let big = Exponent::integer(30000);
        assert_eq!(big.checked_add(big), Err(UnitError::ExponentOverflow));
    }

    #[test]
    fn the_dimensionless_unit_is_the_identity() {
        let one = Unit::dimensionless();
        assert_eq!(m().mul(&one).unwrap(), m());
        assert_eq!(one.mul(&m()).unwrap(), m());
        assert_eq!(one.to_string(), "1");
    }

    // ── The magnitude ──────────────────────────────────────────────────

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n.into(), d.into()).unwrap()
    }

    #[test]
    fn degrees_to_radians_is_a_canonical_datum_not_a_formula() {
        // `37π/180` and `π·37/180` are one value: the coefficient is reduced and the constant
        // carries an exponent, so there is nothing to reorder and equality is syntactic.
        let a = Magnitude::rational(rat(37, 180)).with_constant(Constant::Pi, 1);
        let b = Magnitude::rational(rat(74, 360)).with_constant(Constant::Pi, 1);
        assert_eq!(a, b);
        assert_eq!(a.to_string(), "37/180·π");
    }

    #[test]
    fn a_zero_constant_exponent_is_dropped() {
        let m = Magnitude::rational(rat(1, 2)).with_constant(Constant::Pi, 0);
        assert_eq!(m, Magnitude::rational(rat(1, 2)));
        assert_eq!(m.constant_exponent(Constant::Pi), 0);
    }

    #[test]
    fn a_magnitude_with_a_constant_differs_from_its_coefficient() {
        let with_pi = Magnitude::rational(rat(1, 1)).with_constant(Constant::Pi, 1);
        assert_ne!(with_pi, Magnitude::rational(rat(1, 1)));
    }

    // ── canonical string round-trip (D93 slice 2) ────────────────────────────────────────────

    /// Every canonical form parses back to the value that printed it.
    #[test]
    fn canonical_unit_strings_round_trip() {
        let cases = [
            Unit::dimensionless(),
            Unit::base(BaseDimension::Length),
            Unit::base(BaseDimension::Mass),
            Unit::base(BaseDimension::Length)
                .pow(Exponent::integer(2))
                .unwrap()
                .mul(&Unit::base(BaseDimension::Time).recip().unwrap())
                .unwrap(),
            Unit::base(BaseDimension::Length)
                .pow(Exponent::new(2, 3).unwrap())
                .unwrap(),
        ];
        for u in cases {
            let printed = u.to_canonical_string();
            let back = Unit::parse_canonical(&printed)
                .unwrap_or_else(|e| panic!("{printed:?} did not parse: {e}"));
            assert_eq!(u, back, "round-trip changed the value for {printed:?}");
            assert_eq!(printed, back.to_canonical_string());
        }
    }

    /// The dimensionless unit prints and parses as `1`, not as the empty string.
    #[test]
    fn the_dimensionless_unit_is_spelled_one() {
        assert_eq!(Unit::dimensionless().to_canonical_string(), "1");
        assert_eq!(Unit::parse_canonical("1").unwrap(), Unit::dimensionless());
        assert!(Unit::parse_canonical("").is_err());
    }

    /// A second spelling of a value is REFUSED, not normalised. The content hash runs over the
    /// serialised resource, so accepting both would make two hashes for one value unobservable.
    #[test]
    fn non_canonical_unit_spellings_are_refused() {
        for bad in [
            "m\u{b7}s^-1\u{b7}m",  // unreduced: denotes `s^-1·m^2`
            "m^1",                 // explicit `^1`
            "m^0",                 // a zero exponent has no spelling
            "m^2/1",               // `/1` is the integer form
            "m^02",                // leading zero
            "m^+2",                // leading `+`
            "m\u{b7}m",            // repeated symbol
            "m^2/0",               // zero denominator
            "angle",               // not a base symbol: kinds are not part of a unit
            "s^-1\u{b7}m^2\u{b7}", // trailing separator
            "M",                   // wrong case
            "metre",               // not a symbol
            " m",                  // whitespace
            "1\u{b7}m",            // `1` is not a factor
        ] {
            assert!(
                Unit::parse_canonical(bad).is_err(),
                "{bad:?} should have been refused"
            );
        }
    }

    /// Base symbols print in `BaseDimension::ALL` order — time before length — so a value has one
    /// spelling regardless of how it was built.
    #[test]
    fn base_symbols_print_in_canonical_order() {
        let m2_per_s = Unit::base(BaseDimension::Length)
            .pow(Exponent::integer(2))
            .unwrap()
            .mul(&Unit::base(BaseDimension::Time).recip().unwrap())
            .unwrap();
        let built_other_way = Unit::base(BaseDimension::Time)
            .recip()
            .unwrap()
            .mul(
                &Unit::base(BaseDimension::Length)
                    .pow(Exponent::integer(2))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(m2_per_s.to_canonical_string(), "s^-1\u{b7}m^2");
        assert_eq!(m2_per_s, built_other_way);
    }

    /// serde carries the canonical string and nothing else, so a structural spelling cannot enter
    /// through a deserialiser.
    #[test]
    fn serde_uses_the_canonical_string() {
        let u = Unit::base(BaseDimension::Length)
            .pow(Exponent::integer(2))
            .unwrap();
        let json = serde_json::to_string(&u).unwrap();
        assert_eq!(json, "\"m^2\"");
        assert_eq!(serde_json::from_str::<Unit>(&json).unwrap(), u);

        // A non-canonical spelling is refused at the serde boundary too.
        assert!(serde_json::from_str::<Unit>("\"m\u{b7}m\"").is_err());
    }
}
