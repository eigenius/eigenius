//! Units of measure — the `Unit` primitive's algebra (D93).
//!
//! Three exponent vectors and a coefficient, and no expressions anywhere. That is what keeps
//! symbolic algebra out of the kernel: there is nothing to reorder, so equality is structural.
//!
//! | | structure | kernel operations |
//! |---|---|---|
//! | unit exponents | ℚ⁷ over the seven base dimensions | add, subtract, scalar-multiply, compare |
//! | kind exponents | ℚᴷ over the quantity kinds | the same |
//! | magnitude | ℚ × ℤ^C — a rational coefficient and integer powers of declared constants | **compare only** |
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

/// The seven SI base dimensions. `mol` stays one, against the grain of the kind axis (D93).
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
}

/// A quantity kind — the axis that separates dimensionless quantities from each other.
///
/// It exists because a dimension vector alone unifies `rad`, `sr`, `°`, `%` and `ppm`, and unifying
/// a plane angle with a solid angle is wrong. v1 declares one kind; admitting another is a
/// vocabulary edit, not a checker change.
///
/// `%` and `ppm` are NOT kinds but **scales** on the plain dimensionless unit (D93).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Plane angle. `rad` is `angle¹` and `sr` is `angle²`, so `sr = rad²` falls out rather than
    /// being asserted — a deliberate step past QUDT, which classifies the two without relating
    /// them.
    Angle,
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
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::ExponentOverflow => write!(f, "unit exponent outside the 16-bit range"),
            UnitError::ZeroDenominator => write!(f, "unit exponent has a zero denominator"),
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

/// A unit: a rational exponent vector over the seven base dimensions, plus kind exponents.
///
/// Always canonical. Canonicalisation drops zero exponents, and applies the rule that decides what
/// a kind is for: **kind exponents are carried only when the dimension vector is zero.**
///
/// | | dimension | kind | result |
/// |---|---|---|---|
/// | `rad · rad` | 0 | `angle²` | `sr` — kept, dimension is zero |
/// | `m · rad` (arc length) | `L¹` | discarded | a plain length |
/// | `m / m` | 0 | none | dimensionless, and distinct from `rad` |
///
/// The rule is principled rather than a patch: the kind axis exists *because* dimension cannot
/// separate dimensionless quantities. Once a quantity has a dimension, dimension does the
/// separating and the kind has no work left. Without it a multiplicative kind vector re-breaks
/// `s = rθ`, leaving arc length as a length carrying `angle¹` — which is why an eighth base
/// dimension was rejected.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Unit {
    dimension: [Exponent; 7],
    kinds: BTreeMap<Kind, Exponent>,
}

impl Unit {
    /// The dimensionless unit with no kind — the multiplicative identity.
    pub fn dimensionless() -> Unit {
        Unit {
            dimension: [Exponent::ZERO; 7],
            kinds: BTreeMap::new(),
        }
    }

    /// A base unit raised to the first power.
    pub fn base(d: BaseDimension) -> Unit {
        let mut dimension = [Exponent::ZERO; 7];
        dimension[d as usize] = Exponent::ONE;
        Unit {
            dimension,
            kinds: BTreeMap::new(),
        }
    }

    /// A dimensionless unit carrying one kind — `rad` is `Unit::kind(Kind::Angle, 1)`.
    pub fn kind(k: Kind, power: i16) -> Unit {
        let mut kinds = BTreeMap::new();
        let e = Exponent::integer(power);
        if !e.is_zero() {
            kinds.insert(k, e);
        }
        Unit {
            dimension: [Exponent::ZERO; 7],
            kinds,
        }
    }

    /// The exponent of one base dimension.
    pub fn exponent_of(&self, d: BaseDimension) -> Exponent {
        self.dimension[d as usize]
    }

    /// The exponent of one kind, zero when absent.
    pub fn kind_exponent(&self, k: Kind) -> Exponent {
        self.kinds.get(&k).copied().unwrap_or(Exponent::ZERO)
    }

    /// Whether every base exponent is zero.
    pub fn is_dimensionless(&self) -> bool {
        self.dimension.iter().all(|e| e.is_zero())
    }

    /// Multiplies two units: add the exponents, then canonicalise.
    pub fn mul(&self, other: &Unit) -> Result<Unit, UnitError> {
        let mut dimension = [Exponent::ZERO; 7];
        for (out, (a, b)) in dimension
            .iter_mut()
            .zip(self.dimension.iter().zip(other.dimension.iter()))
        {
            *out = a.checked_add(*b)?;
        }
        let mut kinds = self.kinds.clone();
        for (k, e) in &other.kinds {
            let sum = kinds
                .get(k)
                .copied()
                .unwrap_or(Exponent::ZERO)
                .checked_add(*e)?;
            kinds.insert(*k, sum);
        }
        Ok(canonicalise(dimension, kinds))
    }

    /// Inverts a unit: negate every exponent.
    pub fn recip(&self) -> Result<Unit, UnitError> {
        let mut dimension = [Exponent::ZERO; 7];
        for (out, e) in dimension.iter_mut().zip(self.dimension.iter()) {
            *out = e.checked_neg()?;
        }
        let mut kinds = BTreeMap::new();
        for (k, e) in &self.kinds {
            kinds.insert(*k, e.checked_neg()?);
        }
        Ok(canonicalise(dimension, kinds))
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
        let mut kinds = BTreeMap::new();
        for (k, e) in &self.kinds {
            kinds.insert(*k, e.checked_mul(p)?);
        }
        Ok(canonicalise(dimension, kinds))
    }
}

/// Drops zero exponents, and drops every kind once the dimension vector is non-zero.
fn canonicalise(dimension: [Exponent; 7], kinds: BTreeMap<Kind, Exponent>) -> Unit {
    let dimensionless = dimension.iter().all(|e| e.is_zero());
    let kinds = if dimensionless {
        kinds.into_iter().filter(|(_, e)| !e.is_zero()).collect()
    } else {
        BTreeMap::new()
    };
    Unit { dimension, kinds }
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
        for (k, e) in &self.kinds {
            let name = match k {
                Kind::Angle => "angle",
            };
            parts.push(if *e == Exponent::ONE {
                name.to_string()
            } else {
                format!("{name}^{e}")
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rad() -> Unit {
        Unit::kind(Kind::Angle, 1)
    }
    fn m() -> Unit {
        Unit::base(BaseDimension::Length)
    }
    fn s() -> Unit {
        Unit::base(BaseDimension::Time)
    }

    // ── D93's kind table, the three rows that decide the rule ──────────

    #[test]
    fn rad_times_rad_is_sr_because_the_dimension_is_zero() {
        let sr = rad().mul(&rad()).unwrap();
        assert!(sr.is_dimensionless());
        assert_eq!(sr.kind_exponent(Kind::Angle), Exponent::integer(2));
        // `sr = rad²` falls out rather than being asserted — the step past QUDT.
        assert_eq!(sr, rad().pow(Exponent::integer(2)).unwrap());
    }

    #[test]
    fn metre_times_rad_is_a_plain_length_with_the_kind_discarded() {
        // Arc length: `s = rθ`. Without the discard rule this leaves a length carrying `angle¹`,
        // which is exactly why an eighth base dimension was rejected.
        let arc = m().mul(&rad()).unwrap();
        assert_eq!(arc, m());
        assert_eq!(arc.kind_exponent(Kind::Angle), Exponent::ZERO);
    }

    #[test]
    fn metre_over_metre_is_dimensionless_and_distinct_from_rad() {
        let ratio = m().div(&m()).unwrap();
        assert!(ratio.is_dimensionless());
        assert_eq!(ratio, Unit::dimensionless());
        // The whole reason the kind axis exists: a dimension vector alone would unify these.
        assert_ne!(ratio, rad());
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
}
