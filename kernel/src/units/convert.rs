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

//! Base conversion (D93): an authored value in a stated unit — `5 mg/kg`, `37 °C`, `50 kDa` — into
//! the base-unit magnitude and unit a quantity term carries.
//!
//! **Outside the checker.** D93: "Convenience functions do the mapping … outside the kernel, on the
//! same side of the TCB boundary as D86's literal normalisation … and the kernel checks the result
//! rather than performing it." So this module multiplies magnitudes and nothing in `nbe` can:
//! the arithmetic below is private here, and [`Rational`] itself still has none, which is D94's
//! line — no size-increasing magnitude arithmetic in the checker.
//!
//! **The vocabulary is the chain's.** Symbols, dimensions, factors, prefixes, the °C offset and the
//! kinds are read from the units layer ([`Vocabulary::from_layer`]), never restated in Rust.
//!
//! **One stated form, strictly** (D93 plan, D6.2): factors separated by `·` or spaces, each a symbol
//! with an optional prefix and an optional `^` exponent — a signed integer, or `(p/q)` — and at most
//! one `/`, after which every factor is inverted, as in UCUM. `1` stands for no factors, so `1/s` is
//! a frequency. No `µ` (U+00B5), superscript digits or `per`: D95's unit sub-parser normalises prose
//! into this form and calls the same converter, so there is one.

use super::{Constant, Exponent, Magnitude, Unit, UnitError};
use crate::layer::Layer;
use crate::nbe::term::Exp;
use crate::numeric::{Rational, RationalError};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use std::collections::BTreeMap;
use std::fmt;

const UNITS_NS: &str = "urn:eigenius:units:";

/// The ESL elaboration form `units:quantity(value, "stated")`. NOT a chain constant: the compiler
/// resolves it where the chain's vocabulary is in hand, into [`Converted::quantity_term`]'s term.
pub const QUANTITY_FORM: &str = "urn:eigenius:units:quantity";

/// The inductive a quantity term inhabits, and its one constructor.
pub const QUANTITY: &str = "urn:eigenius:units:Quantity";
const MK_QUANTITY: &str = "mk_quantity";

/// A named unit, as the units layer declares it.
#[derive(Debug, Clone)]
pub struct NamedUnit {
    pub iri: Iri,
    pub symbol: String,
    pub dimension: Unit,
    pub factor: Rational,
    pub factor_pi: i64,
    pub prefixable: bool,
    /// The affine offset in base units — the degree Celsius alone.
    pub offset: Option<Rational>,
    pub kind: Option<Iri>,
}

/// An SI prefix, as the units layer declares it.
#[derive(Debug, Clone)]
pub struct Prefix {
    pub iri: Iri,
    pub symbol: String,
    pub factor: Rational,
}

/// The units layer's vocabulary, keyed by symbol.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    units: BTreeMap<String, NamedUnit>,
    prefixes: BTreeMap<String, Prefix>,
}

/// Why a conversion was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum ConvertError {
    /// The chain carries no usable units layer, or an entry in it is malformed.
    Vocabulary(String),
    /// The stated unit does not parse in the strict form.
    Malformed { stated: String, reason: String },
    /// A token is neither a unit symbol nor a prefix on one.
    UnknownSymbol(String),
    /// A prefix on a unit the SI does not prefix — `kkg`, `mmin`, `k°C`.
    NotPrefixable { token: String, unit: String },
    /// A non-integer exponent whose result is not an exact rational times an integer power of π —
    /// `km^(1/2)` needs √1000.
    Inexact(String),
    /// A unit exponent past its fixed width.
    Unit(UnitError),
    /// A magnitude past D94's bound.
    Rational(RationalError),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Vocabulary(s) => write!(f, "units vocabulary: {s}"),
            ConvertError::Malformed { stated, reason } => {
                write!(f, "stated unit {stated:?} does not parse: {reason}")
            }
            ConvertError::UnknownSymbol(t) => write!(f, "{t:?} is not a unit symbol"),
            ConvertError::NotPrefixable { token, unit } => {
                write!(f, "{token:?}: the SI does not prefix {unit:?}")
            }
            ConvertError::Inexact(s) => write!(f, "{s} has no exact value"),
            ConvertError::Unit(e) => write!(f, "{e}"),
            ConvertError::Rational(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<UnitError> for ConvertError {
    fn from(e: UnitError) -> Self {
        ConvertError::Unit(e)
    }
}

impl From<RationalError> for ConvertError {
    fn from(e: RationalError) -> Self {
        ConvertError::Rational(e)
    }
}

/// The kinds a stated unit's factors carry — METADATA, beside the unit and never in it (D93,
/// "Kinds are metadata, not algebra"). `rad/s` has a plane angle in the numerator; `sr/rad` has a
/// solid angle over a plane angle, which the unit algebra, where both are 1, cannot say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kinds {
    pub numerator: Vec<(Iri, Exponent)>,
    pub denominator: Vec<(Iri, Exponent)>,
}

/// An authored value in base units.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub magnitude: Magnitude,
    pub unit: Unit,
    pub kinds: Kinds,
}

impl Converted {
    /// The quantity term: `(units:mk_quantity(c, pi) : units:Quantity(unit))`.
    ///
    /// Annotated, because `u` is `Quantity`'s parameter and the constructor's arguments do not
    /// carry it: without the annotation the term would inhabit `Quantity(u)` for every `u`. This is
    /// the term `units:quantity(v, "…")` elaborates to in ESL.
    pub fn quantity_term(&self) -> Exp {
        let (coefficient, pi) = self.magnitude.chain_pair();
        let quantity = crate::ontology::well_known::iri(QUANTITY);
        Exp::Ann(
            Box::new(Exp::InductiveCtor(
                quantity.clone(),
                MK_QUANTITY.to_string(),
                vec![Exp::LitRat(coefficient), Exp::LitInt(pi)],
            )),
            Box::new(Exp::const_applied(
                quantity,
                Vec::new(),
                vec![Exp::LitUnit(self.unit.clone())],
            )),
        )
    }
}

/// One parsed factor, its exponent already negated when it followed the `/`.
struct Factor<'v> {
    token: String,
    prefix: Option<&'v Prefix>,
    unit: &'v NamedUnit,
    exp: Exponent,
}

impl Vocabulary {
    /// Reads the units layer's named units and prefixes from the chain `layer` heads.
    pub fn from_layer(layer: &Layer) -> Result<Vocabulary, ConvertError> {
        let mut vocab = Vocabulary::default();
        let named_unit_class = format!("{UNITS_NS}NamedUnit");
        let prefix_class = format!("{UNITS_NS}Prefix");
        for iri in crate::layer::typed_resource_iris(layer, &[named_unit_class.as_str()]) {
            let Some(r) = layer.resolve(&iri) else {
                continue;
            };
            let unit = NamedUnit {
                symbol: string(&r, "symbol")?,
                dimension: Unit::parse_canonical(&string(&r, "dimension")?)?,
                factor: rational(&r, "factor")?,
                factor_pi: r
                    .get(&prop("factor_pi"))
                    .and_then(|v| v.as_integer())
                    .ok_or_else(|| missing(&iri, "factor_pi"))?,
                prefixable: r
                    .get(&prop("prefixable"))
                    .and_then(|v| v.as_boolean())
                    .ok_or_else(|| missing(&iri, "prefixable"))?,
                offset: match r.get(&prop("offset")) {
                    Some(_) => Some(rational(&r, "offset")?),
                    None => None,
                },
                kind: r.get(&prop("kind")).and_then(|v| v.as_iri()),
                iri,
            };
            vocab.units.insert(unit.symbol.clone(), unit);
        }
        for iri in crate::layer::typed_resource_iris(layer, &[prefix_class.as_str()]) {
            let Some(r) = layer.resolve(&iri) else {
                continue;
            };
            let prefix = Prefix {
                symbol: string(&r, "symbol")?,
                factor: rational(&r, "factor")?,
                iri,
            };
            vocab.prefixes.insert(prefix.symbol.clone(), prefix);
        }
        if vocab.units.is_empty() {
            return Err(ConvertError::Vocabulary(
                "the chain carries no units:NamedUnit — is the units layer loaded?".to_string(),
            ));
        }
        Ok(vocab)
    }

    /// The named unit with exactly this symbol.
    pub fn unit(&self, symbol: &str) -> Option<&NamedUnit> {
        self.units.get(symbol)
    }

    /// Converts `value` in the `stated` unit to base units.
    ///
    /// The °C offset applies only when the stated unit is exactly `°C` — the point reading D93
    /// assumes for a bare °C. Inside a compound (`°C/min`) the reading is a difference, and °C
    /// converts as K with no offset: a vector reading carries the vector unit.
    pub fn convert(&self, value: &Rational, stated: &str) -> Result<Converted, ConvertError> {
        let factors = self.parse(stated)?;
        let mut coefficient = value.clone();
        let mut pi = 0i64;
        let mut unit = Unit::dimensionless();
        let mut kinds = Kinds::default();
        for f in &factors {
            let scale = match f.prefix {
                Some(p) => mul(&p.factor, &f.unit.factor)?,
                None => f.unit.factor.clone(),
            };
            coefficient = mul(&coefficient, &pow_exact(&scale, f.exp, &f.token)?)?;
            pi = pi
                .checked_add(integer_times(f.unit.factor_pi, f.exp, &f.token)?)
                .ok_or(ConvertError::Unit(UnitError::ExponentOverflow))?;
            unit = unit.mul(&f.unit.dimension.pow(f.exp)?)?;
            if let Some(kind) = &f.unit.kind {
                let (side, e) = if f.exp.is_positive() {
                    (&mut kinds.numerator, f.exp)
                } else {
                    (&mut kinds.denominator, f.exp.checked_neg()?)
                };
                match side.iter_mut().find(|(k, _)| k == kind) {
                    Some((_, acc)) => *acc = acc.checked_add(e)?,
                    None => side.push((kind.clone(), e)),
                }
            }
        }
        if let [only] = factors.as_slice() {
            if only.prefix.is_none() && only.exp == Exponent::ONE {
                if let Some(offset) = &only.unit.offset {
                    coefficient = add(&coefficient, offset)?;
                }
            }
        }
        let pi = i16::try_from(pi).map_err(|_| ConvertError::Unit(UnitError::ExponentOverflow))?;
        Ok(Converted {
            magnitude: Magnitude::rational(coefficient).with_constant(Constant::Pi, pi),
            unit,
            kinds,
        })
    }

    fn parse(&self, stated: &str) -> Result<Vec<Factor<'_>>, ConvertError> {
        let bad = |reason: &str| ConvertError::Malformed {
            stated: stated.to_string(),
            reason: reason.to_string(),
        };
        if stated.is_empty() || stated.trim() != stated {
            return Err(bad("empty, or leading or trailing whitespace"));
        }
        // The `/` that separates numerator from denominator is the one OUTSIDE parentheses: the
        // exponent in `m^(1/2)` carries one of its own.
        let mut depth = 0i32;
        let mut slashes = Vec::new();
        for (i, ch) in stated.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                '/' if depth == 0 => slashes.push(i),
                _ => {}
            }
        }
        let (num, den) = match slashes.as_slice() {
            [] => (stated, None),
            [i] => (&stated[..*i], Some(&stated[*i + 1..])),
            _ => return Err(bad("more than one `/`")),
        };
        let mut factors = self.product(num, false, &bad)?;
        if let Some(d) = den {
            factors.extend(self.product(d, true, &bad)?);
        }
        Ok(factors)
    }

    /// One side of the `/`: `1`, or factors separated by `·` or spaces.
    fn product(
        &self,
        side: &str,
        inverted: bool,
        bad: &dyn Fn(&str) -> ConvertError,
    ) -> Result<Vec<Factor<'_>>, ConvertError> {
        if side == "1" {
            return Ok(Vec::new());
        }
        if side.is_empty() {
            return Err(bad("an empty side of `/`"));
        }
        let mut out = Vec::new();
        for token in side.split(['\u{b7}', ' ']).filter(|t| !t.is_empty()) {
            let (symbol, exp) = match token.split_once('^') {
                Some((sym, e)) => (sym, parse_exponent(e).ok_or_else(|| bad(token))?),
                None => (token, Exponent::ONE),
            };
            let (prefix, unit) = self.resolve_symbol(symbol)?;
            out.push(Factor {
                token: symbol.to_string(),
                prefix,
                unit,
                exp: if inverted { exp.checked_neg()? } else { exp },
            });
        }
        if out.is_empty() {
            return Err(bad("a side with no factors"));
        }
        Ok(out)
    }

    /// An exact symbol wins; otherwise the LONGEST prefix whose remainder is a unit symbol. So `min`
    /// is the minute, `cd` the candela, `Pa` the pascal, `dam` deca-metre, `ms` millisecond.
    fn resolve_symbol(&self, token: &str) -> Result<(Option<&Prefix>, &NamedUnit), ConvertError> {
        if let Some(u) = self.units.get(token) {
            return Ok((None, u));
        }
        let mut candidates: Vec<&Prefix> = self
            .prefixes
            .values()
            .filter(|p| token.len() > p.symbol.len() && token.starts_with(p.symbol.as_str()))
            .collect();
        candidates.sort_by_key(|p| std::cmp::Reverse(p.symbol.len()));
        for p in candidates {
            if let Some(u) = self.units.get(&token[p.symbol.len()..]) {
                return if u.prefixable {
                    Ok((Some(p), u))
                } else {
                    Err(ConvertError::NotPrefixable {
                        token: token.to_string(),
                        unit: u.symbol.clone(),
                    })
                };
            }
        }
        Err(ConvertError::UnknownSymbol(token.to_string()))
    }
}

fn prop(local: &str) -> Iri {
    crate::ontology::well_known::iri(&format!("{UNITS_NS}{local}"))
}

fn missing(iri: &Iri, local: &str) -> ConvertError {
    ConvertError::Vocabulary(format!("{iri} has no units:{local}"))
}

fn string(r: &Resource, local: &str) -> Result<String, ConvertError> {
    r.get(&prop(local))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ConvertError::Vocabulary(format!("{:?} has no units:{local}", r.id())))
}

fn rational(r: &Resource, local: &str) -> Result<Rational, ConvertError> {
    Ok(Rational::parse_canonical(&string(r, local)?)?)
}

/// `-?digits`, or `(-?digits/digits)`, as an author writes it — reduced, not required canonical.
fn parse_exponent(s: &str) -> Option<Exponent> {
    let (n, d) = match s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => {
            let (n, d) = inner.split_once('/')?;
            (n, d.parse::<i16>().ok()?)
        }
        None => (s, 1),
    };
    let n: i16 = n.parse().ok()?;
    if n == 0 {
        return None;
    }
    Exponent::new(n, d).ok()
}

/// `f × e` for an integer `f` and rational `e`, which must come out integral.
fn integer_times(f: i64, e: Exponent, token: &str) -> Result<i64, ConvertError> {
    let r = e.to_rational();
    let product = BigInt::from(f) * r.numer();
    if (&product % r.denom()).is_zero() {
        i64::try_from(product / r.denom())
            .map_err(|_| ConvertError::Unit(UnitError::ExponentOverflow))
    } else {
        Err(ConvertError::Inexact(format!(
            "{token}^{e}: π to a non-integer power"
        )))
    }
}

fn mul(a: &Rational, b: &Rational) -> Result<Rational, ConvertError> {
    Ok(Rational::new(a.numer() * b.numer(), a.denom() * b.denom())?)
}

fn add(a: &Rational, b: &Rational) -> Result<Rational, ConvertError> {
    Ok(Rational::new(
        a.numer() * b.denom() + b.numer() * a.denom(),
        a.denom() * b.denom(),
    )?)
}

/// `base^e` exactly: an integer power, or a root that comes out rational.
fn pow_exact(base: &Rational, e: Exponent, token: &str) -> Result<Rational, ConvertError> {
    let r = e.to_rational();
    let (n, d) = (r.numer().clone(), r.denom().clone());
    let d: u32 = u32::try_from(d).map_err(|_| ConvertError::Unit(UnitError::ExponentOverflow))?;
    let root = |x: &BigInt| -> Option<BigInt> {
        if d == 1 {
            return Some(x.clone());
        }
        if x.is_negative() {
            return None;
        }
        let y = x.nth_root(d);
        (num_traits::pow(y.clone(), d as usize) == *x).then_some(y)
    };
    let inexact = || ConvertError::Inexact(format!("{token}^{e} of {base}"));
    let (bn, bd) = (
        root(base.numer()).ok_or_else(inexact)?,
        root(base.denom()).ok_or_else(inexact)?,
    );
    let k =
        usize::try_from(n.abs()).map_err(|_| ConvertError::Unit(UnitError::ExponentOverflow))?;
    let (pn, pd) = (num_traits::pow(bn, k), num_traits::pow(bd, k));
    let (num, den) = if n.is_negative() { (pd, pn) } else { (pn, pd) };
    if num.is_zero() && den.is_one() {
        return Ok(Rational::from_integer(BigInt::zero())?);
    }
    Ok(Rational::new(num, den)?)
}
