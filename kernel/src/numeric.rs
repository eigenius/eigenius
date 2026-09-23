//! Exact rational numbers — the carrier for `core:rational` (D94).
//!
//! A [`Rational`] is always in canonical form: reduced, with a positive denominator and the sign on
//! the numerator. The constructor ESTABLISHES that rather than checking it, which is what lets
//! definitional equality compare two rationals structurally — `conv` does no arithmetic, because
//! two canonical forms are equal iff their components are.
//!
//! The same invariant fixes the serialised form. D94 lowers a rational to a canonical decimal
//! string carried as `Value::String`, because JSON numbers are IEEE doubles and an exact value does
//! not survive them: the exact `0.05` is `3602879701896397 / 2^56`, whose denominator already
//! exceeds the 53-bit safe range `ontology::Value::Integer` documents. Canonical form is what makes
//! string equality value equality, so no rational-specific arm is needed where values are compared.
//!
//! Magnitudes are never computed on (D94, "The magnitude bound"): conversion happens at ingest,
//! outside the kernel, so the kernel only ever compares. That is what makes [`MAX_BITS`] hold — a
//! literal bound constrains nothing if a multiplication can double a width.

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::fmt;

/// Maximum width of a numerator or denominator, in bits.
///
/// The bound is for **termination, not precision** (D94): it exists so an authored term cannot ask
/// the checker for unbounded work. The largest identified requirement is a binary64-derived
/// denominator (2^1074) composed with a quecto prefix (10^-30), about 1175 bits; 4096 clears that
/// by more than triple and still refuses `10^1000000` long before anything threatens termination.
pub const MAX_BITS: u64 = 4096;

/// The largest power of ten a bounded value can carry: `10^1233` is 4096 bits, `10^1234` is 4099.
///
/// Checked BEFORE the power is computed. [`MAX_BITS`] exists so an authored term cannot ask the
/// checker for unbounded work (D94), and computing `10^999999999` to discover it is too large does
/// exactly that work — the check has to come first or it is not a bound.
const MAX_DECIMAL_EXPONENT: u32 = 1233;

/// Why a rational could not be admitted.
///
/// Every variant REFUSES. Nothing here truncates, rounds or normalises a malformed input into a
/// well-formed one, because an exact value that was silently adjusted is worse than none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RationalError {
    /// A denominator of zero. Not a number, so not a refusal about size.
    ZeroDenominator,
    /// A numerator or denominator wider than [`MAX_BITS`].
    TooLarge {
        /// Which component was too wide.
        component: Component,
        /// Its actual width in bits.
        bits: u64,
    },
    /// The string is not in the canonical grammar.
    Malformed(String),
}

/// Which half of a rational a [`RationalError::TooLarge`] refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    /// The numerator.
    Numerator,
    /// The denominator.
    Denominator,
}

impl fmt::Display for RationalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RationalError::ZeroDenominator => write!(f, "rational has a zero denominator"),
            RationalError::TooLarge { component, bits } => {
                let which = match component {
                    Component::Numerator => "numerator",
                    Component::Denominator => "denominator",
                };
                write!(
                    f,
                    "rational {which} is {bits} bits, over the {MAX_BITS}-bit bound"
                )
            }
            RationalError::Malformed(s) => {
                write!(f, "rational literal `{s}` is not in canonical form")
            }
        }
    }
}

impl std::error::Error for RationalError {}

/// An exact rational in canonical form.
///
/// Canonical means: reduced (`gcd(num, den) == 1`), `den > 0`, sign on the numerator, and `den == 1`
/// for an integer. Constructing one is the only way to get one, so no `Rational` in the system is
/// ever non-canonical.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rational(BigRational);

impl Rational {
    /// Builds a rational from a numerator and denominator, reducing and bounding it.
    ///
    /// Reduction happens here — the gcd is paid once, at admission, which D94 settles as the only
    /// place it is paid: verifying canonical form has no cheaper check than computing it.
    pub fn new(numer: BigInt, denom: BigInt) -> Result<Self, RationalError> {
        if denom.is_zero() {
            return Err(RationalError::ZeroDenominator);
        }
        // Bound the INPUTS as well as the reduced result: `10^5000 / 10^5000` reduces to 1, and
        // admitting it would mean the checker had already done the work the bound exists to refuse.
        check_bits(&numer, Component::Numerator)?;
        check_bits(&denom, Component::Denominator)?;
        // `Ratio::new` reduces and moves the sign to the numerator, which is the canonical form.
        let r = BigRational::new(numer, denom);
        check_bits(r.numer(), Component::Numerator)?;
        check_bits(r.denom(), Component::Denominator)?;
        Ok(Rational(r))
    }

    /// Builds a rational from an integer, with denominator 1.
    pub fn from_integer(numer: BigInt) -> Result<Self, RationalError> {
        check_bits(&numer, Component::Numerator)?;
        Ok(Rational(BigRational::from_integer(numer)))
    }

    /// The numerator, carrying the sign.
    pub fn numer(&self) -> &BigInt {
        self.0.numer()
    }

    /// The denominator, always positive.
    pub fn denom(&self) -> &BigInt {
        self.0.denom()
    }

    /// Whether this rational is an integer — `den == 1`.
    ///
    /// This is the check `core:bigint` makes as a refinement of `core:rational`, the relationship
    /// `PrimitiveType::Iri` has to `String` (D88 §3).
    pub fn is_integer(&self) -> bool {
        self.0.denom().is_one()
    }

    /// Parses the canonical decimal form, refusing anything else.
    ///
    /// The grammar is `'-'? uint ('/' uint)?` with `uint` being `'0'` or `[1-9][0-9]*`. Leading
    /// zeros, a leading `+`, whitespace, a negative denominator, an unreduced pair and an explicit
    /// `/1` are all REFUSED rather than normalised: the content hash runs over the serialised
    /// resource, so two spellings of one value would hash differently and a parser that accepted
    /// both would make that unobservable.
    pub fn parse_canonical(s: &str) -> Result<Self, RationalError> {
        let bad = || RationalError::Malformed(s.to_string());
        let (num_str, den_str) = match s.split_once('/') {
            Some((n, d)) => (n, Some(d)),
            None => (s, None),
        };
        let (negative, digits) = match num_str.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, num_str),
        };
        if !is_canonical_uint(digits) {
            return Err(bad());
        }
        if negative && digits == "0" {
            // `-0` is a second spelling of zero.
            return Err(bad());
        }
        let mut numer: BigInt = digits.parse().map_err(|_| bad())?;
        if negative {
            numer = -numer;
        }
        let denom = match den_str {
            None => BigInt::one(),
            Some(d) => {
                if !is_canonical_uint(d) || d == "0" || d == "1" {
                    // `/0` is not a number and `/1` is a second spelling of the integer form.
                    return Err(bad());
                }
                d.parse().map_err(|_| bad())?
            }
        };
        let value = Rational::new(numer, denom)?;
        // Reject an unreduced input rather than silently reducing it: `2/4` and `1/2` are one value
        // with two spellings, and only one of them is admissible.
        if value.to_canonical_string() != s {
            return Err(bad());
        }
        Ok(value)
    }

    /// Parses a decimal literal EXACTLY — `0.05` is 1/20, not 3602879701896397/2^56.
    ///
    /// Accepts the spellings a source text uses: `37`, `0.05`, `5e-2`, `1.6e-19`, with an optional
    /// leading `-`. They denote one value, not three approximations: an exact rational has no
    /// lattice of representable values, so the radix has no semantic role here, only a surface one
    /// (D94, "Not open either: the radix").
    ///
    /// This is the conversion behind the `r` suffix. Without it `0.05r` would have to be written
    /// `1/20` to be exact, and a decimal surface where `0.05` silently meant the binary64 value
    /// would be the trap the suffix exists to close.
    pub fn parse_decimal(s: &str) -> Result<Self, RationalError> {
        let bad = || RationalError::Malformed(s.to_string());
        let (negative, rest) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (mantissa, exp) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => {
                let e: i32 = e.parse().map_err(|_| bad())?;
                (m, e)
            }
            None => (rest, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, f)) => (i, f),
            None => (mantissa, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(bad());
        }
        let digits = format!("{int_part}{frac_part}");
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        // Before parsing: a decimal with more digits than the bound allows cannot be admitted, and
        // reading it into a bignum first would be the unbounded work the bound forbids.
        if digits.len() > MAX_DECIMAL_EXPONENT as usize + 1 {
            return Err(RationalError::TooLarge {
                component: Component::Numerator,
                bits: MAX_BITS + 1,
            });
        }
        let mut numer: BigInt = digits.parse().map_err(|_| bad())?;
        if negative {
            numer = -numer;
        }
        // The value is `digits × 10^(exp - frac_len)`. A negative power becomes the denominator.
        let scale = exp - i32::try_from(frac_part.len()).map_err(|_| bad())?;
        let ten = BigInt::from(10);
        let magnitude = scale.unsigned_abs();
        if magnitude > MAX_DECIMAL_EXPONENT {
            return Err(RationalError::TooLarge {
                component: if scale >= 0 {
                    Component::Numerator
                } else {
                    Component::Denominator
                },
                bits: MAX_BITS + 1,
            });
        }
        if scale >= 0 {
            Rational::from_integer(numer * ten.pow(magnitude))
        } else {
            Rational::new(numer, ten.pow(magnitude))
        }
    }

    /// Renders the canonical decimal form: `num`, or `num/den` when the denominator is not 1.
    pub fn to_canonical_string(&self) -> String {
        if self.is_integer() {
            self.0.numer().to_string()
        } else {
            format!("{}/{}", self.0.numer(), self.0.denom())
        }
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

/// Serialises as the canonical decimal STRING, never as a number.
///
/// A JSON number is an IEEE double, so an exact value does not survive one. Making this the only
/// serde representation means every path through serde is canonical by construction rather than by
/// each call site remembering (D94).
impl serde::Serialize for Rational {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_canonical_string())
    }
}

/// Deserialises from the canonical form, REFUSING any other spelling.
///
/// `2/4` and `01` are numbers, and are still rejected: two spellings of one value would hash
/// differently, and the content hash runs over the serialised resource.
impl<'de> serde::Deserialize<'de> for Rational {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(de)?;
        Rational::parse_canonical(&s).map_err(serde::de::Error::custom)
    }
}

fn check_bits(v: &BigInt, component: Component) -> Result<(), RationalError> {
    let bits = v.abs().bits();
    if bits > MAX_BITS {
        return Err(RationalError::TooLarge { component, bits });
    }
    Ok(())
}

/// Whether `s` is `0` or a digit string with no leading zero.
fn is_canonical_uint(s: &str) -> bool {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    s == "0" || !s.starts_with('0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_traits::Pow;

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(BigInt::from(n), BigInt::from(d)).expect("admissible")
    }

    #[test]
    fn constructor_reduces() {
        assert_eq!(r(2, 4).to_canonical_string(), "1/2");
        assert_eq!(r(6, 3).to_canonical_string(), "2");
    }

    #[test]
    fn sign_moves_to_the_numerator() {
        assert_eq!(r(1, -2).to_canonical_string(), "-1/2");
        assert_eq!(r(-1, -2).to_canonical_string(), "1/2");
        assert!(r(1, -2).denom() > &BigInt::from(0));
    }

    #[test]
    fn equal_values_built_differently_compare_equal() {
        // The property `conv` rests on: canonical form makes structural equality value equality,
        // so definitional equality needs no arithmetic.
        assert_eq!(r(1, 2), r(50, 100));
        assert_eq!(r(-3, 6), r(3, -6));
    }

    #[test]
    fn an_integer_prints_without_a_denominator() {
        assert_eq!(r(7, 1).to_canonical_string(), "7");
        assert!(r(7, 1).is_integer());
        assert!(!r(7, 2).is_integer());
    }

    #[test]
    fn zero_denominator_is_refused() {
        let e = Rational::new(BigInt::from(1), BigInt::from(0)).unwrap_err();
        assert_eq!(e, RationalError::ZeroDenominator);
    }

    #[test]
    fn canonical_strings_round_trip() {
        for s in [
            "0",
            "1",
            "-1",
            "1/2",
            "-1/2",
            "3602879701896397/72057594037927936",
        ] {
            let v = Rational::parse_canonical(s).expect(s);
            assert_eq!(v.to_canonical_string(), s);
        }
    }

    #[test]
    fn non_canonical_strings_are_refused_not_normalised() {
        // Each is a second spelling of a value that already has a canonical one. Admitting any
        // would let two serialisations of one value hash differently.
        for s in [
            "01", "+1", " 1", "1 ", "2/4", "1/1", "-0", "1/-2", "1/0", "", "-", "1/", "/2", "1.5",
        ] {
            assert!(
                Rational::parse_canonical(s).is_err(),
                "expected `{s}` to be refused"
            );
        }
    }

    #[test]
    fn decimal_spellings_denote_one_exact_value() {
        // D94: `0.05`, `5e-2` and `1/20` are three spellings of one value, not three
        // approximations. Contrast the binary64 reading below, which is a different number.
        for s in ["0.05", "5e-2", "0.0500", "5E-2", "500e-4"] {
            assert_eq!(
                Rational::parse_decimal(s).expect(s).to_canonical_string(),
                "1/20",
                "for `{s}`"
            );
        }
    }

    #[test]
    fn parse_decimal_is_exact_where_binary64_is_not() {
        // The whole reason for the suffix: these are different numbers.
        let exact = Rational::parse_decimal("0.05").unwrap();
        let binary64 = Rational::new(
            BigInt::from(3602879701896397i64),
            BigInt::from(2).pow(56u32),
        )
        .unwrap();
        assert_ne!(exact, binary64);
        assert_eq!(exact.to_canonical_string(), "1/20");
    }

    #[test]
    fn parse_decimal_handles_signs_integers_and_exponents() {
        assert_eq!(
            Rational::parse_decimal("37").unwrap().to_canonical_string(),
            "37"
        );
        assert_eq!(
            Rational::parse_decimal("-0.5")
                .unwrap()
                .to_canonical_string(),
            "-1/2"
        );
        assert_eq!(
            Rational::parse_decimal("1.5e2")
                .unwrap()
                .to_canonical_string(),
            "150"
        );
        assert_eq!(
            Rational::parse_decimal("0").unwrap().to_canonical_string(),
            "0"
        );
    }

    /// The bound has to be checked BEFORE the power is computed, or it is not a bound: `10^1e9`
    /// is a 3.3-billion-bit integer — about 415 MB, and some thirty squarings to build — and an
    /// earlier version of `parse_decimal` constructed it before discovering it was too large.
    #[test]
    fn a_huge_exponent_is_refused_without_computing_it() {
        let start = std::time::Instant::now();
        for s in ["1e999999999", "1e-999999999", "1e2000", "1e-2000"] {
            assert!(
                matches!(
                    Rational::parse_decimal(s),
                    Err(RationalError::TooLarge { .. })
                ),
                "expected `{s}` to be refused as too large"
            );
        }
        // Generous, but four exponentiations at that size would not finish this side of a year.
        assert!(
            start.elapsed().as_secs() < 5,
            "the bound was checked after the work, not before"
        );
    }

    #[test]
    fn the_largest_admissible_power_of_ten_still_parses() {
        // 10^1233 is 4096 bits; 10^1234 is 4099.
        assert!(Rational::parse_decimal(&format!("1e{MAX_DECIMAL_EXPONENT}")).is_ok());
        assert!(Rational::parse_decimal(&format!("1e{}", MAX_DECIMAL_EXPONENT + 1)).is_err());
    }

    #[test]
    fn parse_decimal_refuses_what_is_not_a_decimal() {
        for s in ["", ".", "1.2.3", "1e", "abc", "1/2"] {
            assert!(
                Rational::parse_decimal(s).is_err(),
                "expected `{s}` to be refused"
            );
        }
    }

    #[test]
    fn the_exact_binary64_nought_point_nought_five() {
        // D94's motivating value: 0.05 as a binary64 is 3602879701896397 / 2^56, whose denominator
        // (~7.2e16) exceeds the 53-bit range `ontology::Value::Integer` documents. It is exactly
        // why an exact value cannot ride in a JSON number.
        let den = BigInt::from(2).pow(56u32);
        let v = Rational::new(BigInt::from(3602879701896397i64), den).unwrap();
        assert_eq!(
            v.to_canonical_string(),
            "3602879701896397/72057594037927936"
        );
        assert!(!v.is_integer());
    }

    #[test]
    fn the_ev_denominator_is_admissible() {
        // 1 eV = 1.602176634e-19 J needs a denominator of 10^28 — 93 bits, far inside the bound,
        // and far outside a JSON number.
        let v = Rational::new(BigInt::from(1602176634i64), BigInt::from(10).pow(28u32)).unwrap();
        assert!(v.denom().bits() > 53);
        assert!(v.denom().bits() < MAX_BITS);
    }

    #[test]
    fn the_bound_admits_at_the_limit_and_refuses_past_it() {
        // 2^4095 is 4096 bits; 2^4096 is 4097.
        let at = BigInt::from(2).pow(4095u32);
        assert_eq!(at.bits(), MAX_BITS);
        assert!(Rational::from_integer(at).is_ok());

        let over = BigInt::from(2).pow(4096u32);
        assert_eq!(over.bits(), MAX_BITS + 1);
        match Rational::from_integer(over).unwrap_err() {
            RationalError::TooLarge { component, bits } => {
                assert_eq!(component, Component::Numerator);
                assert_eq!(bits, MAX_BITS + 1);
            }
            e => panic!("expected TooLarge, got {e:?}"),
        }
    }

    #[test]
    fn the_bound_applies_to_inputs_not_only_to_the_reduced_result() {
        // `10^5000 / 10^5000` reduces to 1. Admitting it would mean the checker had already done
        // the work the bound exists to refuse.
        let big = BigInt::from(10).pow(5000u32);
        assert!(Rational::new(big.clone(), big).is_err());
    }

    #[test]
    fn the_denominator_is_bounded_too() {
        let over = BigInt::from(2).pow(4096u32);
        match Rational::new(BigInt::from(1), over).unwrap_err() {
            RationalError::TooLarge { component, .. } => {
                assert_eq!(component, Component::Denominator);
            }
            e => panic!("expected TooLarge, got {e:?}"),
        }
    }
}
