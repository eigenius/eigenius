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

//! D95 — reading a unit as prose writes it, after a numeral.
//!
//! The units layer's symbols are strict: one per unit, and the ESL form `units:quantity(5, "mg/kg")`
//! resolves against them alone (D93). Prose is looser. `g` is the gram and, in `931g`, standard
//! gravity; `ml` is a millilitre though the litre's symbol is `L`; `μg ml⁻¹` is a quotient written as
//! a product with a superscript; `µ` (U+00B5) stands for `μ`. [`ProseUnits`] reads such an
//! expression into every reading it has, restates each in the strict form, and converts it with
//! [`Vocabulary::convert`] — one converter for both (D95, "Carried in from D93").
//!
//! What reads as a unit, after a numeral and optional whitespace:
//!
//! - **a factor** — a run of letters (`°`, `˚`, `℃` count as letters here), optionally with an
//!   exponent (`²`, `⁻¹`, `^2`, `^-1`, `^(1/2)`), and not followed by a letter or digit. The spelling
//!   resolves exact-first, then as the longest prefix on a spelling whose unit takes prefixes, to
//!   every unit it can denote. `′` and `″` are not unit characters: after a numeral they write DNA
//!   ends (`5′`, `3′`), not arcminutes. A closed-class word (`at`, `as`, `in`) is never a factor,
//!   though `at` spells the attotonne.
//! - **more factors** — after `·`, or after whitespace when the factor has an exponent (`μg ml⁻¹`).
//!   A bare word after whitespace ends the expression, so `2 h at 37 °C` reads `h`.
//! - **one quotient** — `/`, or ` per `, after which the factors are inverted.
//! - **`%`** alone — a number notation, 1/100, not a unit (D93).
//!
//! The longest expression with at least one reading wins.

use std::collections::BTreeMap;

use crate::layer::Layer;
use crate::numeric::Rational;
use crate::ontology::iri::Iri;
use crate::units::convert::{Converted, Reading, Vocabulary};
use crate::units::Exponent;

use super::closed_class::is_closed_class_surface;

/// A quantity token's content: the numeral as written, and every way to read its unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Quantity {
    pub value: Rational,
    pub readings: Vec<UnitReading>,
}

/// One reading of a unit expression: a unit for each factor, restated in D93's strict form and
/// converted to base units.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitReading {
    /// The named unit each factor was read as, in order. Empty for `%`.
    pub units: Vec<Iri>,
    /// The strict stated form the reading converted: `μg·mL^-1`, `g_n`; `1` for `%`, whose 1/100 is
    /// applied to the value.
    pub stated: String,
    /// The quantity in base units, as a measured value — a bare °C with its offset (D93).
    pub value: Converted,
    /// The same, as a difference — never with the offset (D95, decision 5).
    pub difference: Converted,
}

/// A unit a spelling can denote.
#[derive(Debug, Clone)]
struct Sense {
    iri: Iri,
    /// The unit's strict symbol.
    symbol: String,
    prefixable: bool,
}

/// The unit vocabulary as prose spells it: the units layer's strict symbols, and the
/// `lexicon:UnitSurface` spellings the lexicon adds (D95, decision 2). Case-sensitive.
#[derive(Debug, Clone, Default)]
pub struct ProseUnits {
    vocabulary: Option<Vocabulary>,
    spellings: BTreeMap<String, Vec<Sense>>,
    /// Prefix symbols, longest first.
    prefixes: Vec<String>,
}

const UNIT_SURFACE: &str = "urn:eigenius:lexicon:UnitSurface";
const UNIT_FORM: &str = "urn:eigenius:lexicon:unit_form";
const UNIT: &str = "urn:eigenius:lexicon:unit";

impl ProseUnits {
    /// No vocabulary: nothing reads as a unit.
    pub fn none() -> ProseUnits {
        ProseUnits::default()
    }

    /// The vocabulary the chain `layer` heads. A chain without the units layer reads no units.
    pub fn load(layer: &Layer) -> ProseUnits {
        let Ok(vocabulary) = Vocabulary::from_layer(layer) else {
            return ProseUnits::none();
        };
        let mut spellings: BTreeMap<String, Vec<Sense>> = BTreeMap::new();
        let mut by_iri: BTreeMap<Iri, Sense> = BTreeMap::new();
        for u in vocabulary.units() {
            let sense = Sense {
                iri: u.iri.clone(),
                symbol: u.symbol.clone(),
                prefixable: u.prefixable,
            };
            spellings
                .entry(u.symbol.clone())
                .or_default()
                .push(sense.clone());
            by_iri.insert(u.iri.clone(), sense);
        }
        let (form, unit) = (
            crate::ontology::well_known::iri(UNIT_FORM),
            crate::ontology::well_known::iri(UNIT),
        );
        for iri in crate::layer::typed_resource_iris(layer, &[UNIT_SURFACE]) {
            let Some(r) = layer.resolve(&iri) else {
                continue;
            };
            let spelling = r.get(&form).and_then(|v| v.as_str()).map(str::to_string);
            let sense = r
                .get(&unit)
                .and_then(|v| v.as_iri())
                .and_then(|u| by_iri.get(&u));
            if let (Some(spelling), Some(sense)) = (spelling, sense) {
                spellings.entry(spelling).or_default().push(sense.clone());
            }
        }
        let mut prefixes: Vec<String> = vocabulary.prefixes().map(|p| p.symbol.clone()).collect();
        prefixes.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
        ProseUnits {
            vocabulary: Some(vocabulary),
            spellings,
            prefixes,
        }
    }

    /// Reads a unit expression in `text` from byte `at`, for the numeral `value`: directly when
    /// `attached` (`5mg`), otherwise after optional whitespace (`37 °C`, `10%`). Returns where the
    /// longest expression with a reading ends, and its readings; `None` when nothing there reads as
    /// a unit.
    pub fn read(
        &self,
        text: &str,
        at: usize,
        attached: bool,
        value: &Rational,
    ) -> Option<(usize, Vec<UnitReading>)> {
        let vocabulary = self.vocabulary.as_ref()?;
        let start = if attached { at } else { skip_space(text, at) };
        if text[start..].starts_with('%') {
            let scaled = Rational::new(
                value.numer().clone(),
                value.denom() * num_bigint::BigInt::from(100),
            )
            .ok()?;
            let value = vocabulary.convert_as(&scaled, "1", Reading::Value).ok()?;
            let difference = vocabulary
                .convert_as(&scaled, "1", Reading::Difference)
                .ok()?;
            return Some((
                start + '%'.len_utf8(),
                vec![UnitReading {
                    units: Vec::new(),
                    stated: "1".to_string(),
                    value,
                    difference,
                }],
            ));
        }
        let candidates = scan(text, start);
        candidates.into_iter().rev().find_map(|(end, factors)| {
            let readings = self.readings(vocabulary, &factors, value);
            (!readings.is_empty()).then_some((end, readings))
        })
    }

    /// Every sense combination of `factors` that converts.
    fn readings(
        &self,
        vocabulary: &Vocabulary,
        factors: &[Scanned],
        value: &Rational,
    ) -> Vec<UnitReading> {
        let mut per_factor: Vec<Vec<(String, Iri)>> = Vec::with_capacity(factors.len());
        for f in factors {
            let senses = self.senses(&f.spelling);
            if senses.is_empty() {
                return Vec::new();
            }
            let exp = if f.inverted {
                match f.exp.checked_neg() {
                    Ok(e) => e,
                    Err(_) => return Vec::new(),
                }
            } else {
                f.exp
            };
            per_factor.push(
                senses
                    .into_iter()
                    .map(|(prefix, sense)| {
                        let strict = format!("{}{}{}", prefix, sense.symbol, exponent_suffix(exp));
                        (strict, sense.iri.clone())
                    })
                    .collect(),
            );
        }
        let mut combos: Vec<(Vec<String>, Vec<Iri>)> = vec![(Vec::new(), Vec::new())];
        for options in &per_factor {
            combos = combos
                .into_iter()
                .flat_map(|(strict, iris)| {
                    options.iter().map(move |(s, i)| {
                        let mut strict = strict.clone();
                        let mut iris = iris.clone();
                        strict.push(s.clone());
                        iris.push(i.clone());
                        (strict, iris)
                    })
                })
                .collect();
        }
        combos
            .into_iter()
            .filter_map(|(strict, units)| {
                let stated = strict.join("·");
                let converted = vocabulary.convert_as(value, &stated, Reading::Value).ok()?;
                let difference = vocabulary
                    .convert_as(value, &stated, Reading::Difference)
                    .ok()?;
                Some(UnitReading {
                    units,
                    stated,
                    value: converted,
                    difference,
                })
            })
            .collect()
    }

    /// The units `spelling` can denote, each with the strict prefix symbol it carries (`""` for
    /// none): an exact spelling's every unit, or else the longest prefix on a spelling whose units take
    /// prefixes. `µ` (U+00B5) reads as `μ`.
    fn senses(&self, spelling: &str) -> Vec<(&str, &Sense)> {
        if is_closed_class_surface(spelling) {
            return Vec::new();
        }
        let spelling = spelling.replace('µ', "μ");
        if let Some(senses) = self.spellings.get(&spelling) {
            return senses.iter().map(|s| ("", s)).collect();
        }
        for p in &self.prefixes {
            let Some(rest) = spelling.strip_prefix(p.as_str()) else {
                continue;
            };
            if let Some(senses) = self.spellings.get(rest) {
                return senses
                    .iter()
                    .filter(|s| s.prefixable)
                    .map(|s| (p.as_str(), s))
                    .collect();
            }
        }
        Vec::new()
    }
}

/// `^n`, `^(p/q)`, or nothing for 1.
fn exponent_suffix(exp: Exponent) -> String {
    if exp == Exponent::ONE {
        return String::new();
    }
    let r = exp.to_rational();
    if r.is_integer() {
        format!("^{}", r.numer())
    } else {
        format!("^({}/{})", r.numer(), r.denom())
    }
}

/// One factor as written.
#[derive(Debug, Clone)]
struct Scanned {
    spelling: String,
    exp: Exponent,
    /// After the `/` or `per`.
    inverted: bool,
}

fn skip_space(text: &str, at: usize) -> usize {
    at + text[at..]
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map_or(text.len() - at, |(i, _)| i)
}

fn is_unit_char(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '°' | '˚' | '℃')
}

fn superscript_digit(c: char) -> Option<u32> {
    match c {
        '⁰' => Some(0),
        '¹' => Some(1),
        '²' => Some(2),
        '³' => Some(3),
        '⁴'..='⁹' => Some(c as u32 - '⁴' as u32 + 4),
        _ => None,
    }
}

/// Every prefix of the expression at `at` that ends after a whole factor, with its factors, in
/// order of length.
fn scan(text: &str, at: usize) -> Vec<(usize, Vec<Scanned>)> {
    let mut out = Vec::new();
    let mut factors: Vec<Scanned> = Vec::new();
    let mut inverted = false;
    let mut pos = at;
    // The first factor needs no exponent.
    let mut need_exponent = false;
    while let Some((spelling, exp, explicit, end)) = factor(text, pos) {
        if need_exponent && !explicit {
            break;
        }
        factors.push(Scanned {
            spelling,
            exp,
            inverted,
        });
        pos = end;
        out.push((pos, factors.clone()));

        // What joins the next factor.
        let rest = &text[pos..];
        if let Some(r) = rest.strip_prefix('·').or_else(|| rest.strip_prefix('⋅')) {
            pos = text.len() - r.len();
            need_exponent = false;
            continue;
        }
        let after = skip_space(text, pos);
        if text[after..].starts_with('/') && !inverted {
            inverted = true;
            pos = skip_space(text, after + 1);
            need_exponent = false;
            continue;
        }
        if after > pos {
            if let Some(r) = text[after..].strip_prefix("per") {
                let past = text.len() - r.len();
                if !inverted && skip_space(text, past) > past {
                    inverted = true;
                    pos = skip_space(text, past);
                    need_exponent = false;
                    continue;
                }
            }
            // A space-separated factor must carry its exponent: otherwise the next English word
            // would be read as a unit.
            pos = after;
            need_exponent = true;
            continue;
        }
        break;
    }
    out
}

/// A factor at `at`: its spelling, its exponent, whether the exponent was written, and where it ends.
/// `None` if no factor starts there, or one runs on into a letter or digit.
fn factor(text: &str, at: usize) -> Option<(String, Exponent, bool, usize)> {
    let rest = &text[at..];
    let len: usize = rest
        .chars()
        .take_while(|c| is_unit_char(*c))
        .map(char::len_utf8)
        .sum();
    if len == 0 {
        return None;
    }
    let spelling = rest[..len].to_string();
    let (exp, explicit, end) = match exponent(text, at + len) {
        Some((exp, end)) => (exp, true, end),
        None => (Exponent::ONE, false, at + len),
    };
    if text[end..]
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    Some((spelling, exp, explicit, end))
}

/// An exponent at `at`: superscript (`²`, `⁻¹`) or caret (`^2`, `^-1`, `^−1`, `^(1/2)`).
fn exponent(text: &str, at: usize) -> Option<(Exponent, usize)> {
    let rest = &text[at..];
    // Superscript.
    let (negative, digits_from) = match rest.strip_prefix('⁻') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let mut n: i64 = 0;
    let mut used = 0;
    for c in digits_from.chars() {
        let Some(d) = superscript_digit(c) else {
            break;
        };
        n = n.checked_mul(10)?.checked_add(i64::from(d))?;
        used += c.len_utf8();
    }
    if used > 0 {
        let n = i16::try_from(if negative { -n } else { n }).ok()?;
        let end = at + (rest.len() - digits_from.len()) + used;
        return (n != 0)
            .then(|| Exponent::new(n, 1).ok())
            .flatten()
            .map(|e| (e, end));
    }
    // Caret.
    let r = rest.strip_prefix('^')?;
    if let Some(inner) = r.strip_prefix('(') {
        let close = inner.find(')')?;
        let (p, q) = inner[..close].split_once('/')?;
        let p: i16 = p.replace('−', "-").parse().ok()?;
        let q: i16 = q.parse().ok()?;
        let end = at + 1 + 1 + close + 1;
        return (p != 0)
            .then(|| Exponent::new(p, q).ok())
            .flatten()
            .map(|e| (e, end));
    }
    let (negative, digits) = match r.strip_prefix('-').or_else(|| r.strip_prefix('−')) {
        Some(d) => (true, d),
        None => (false, r),
    };
    let len = digits.bytes().take_while(u8::is_ascii_digit).count();
    if len == 0 {
        return None;
    }
    let n: i16 = digits[..len].parse().ok()?;
    let n = if negative { -n } else { n };
    let end = at + (rest.len() - digits.len()) + len;
    (n != 0)
        .then(|| Exponent::new(n, 1).ok())
        .flatten()
        .map(|e| (e, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_factors_and_their_joins() {
        let spellings = |text: &str| -> Vec<Vec<(String, bool)>> {
            scan(text, 0)
                .into_iter()
                .map(|(_, fs)| fs.into_iter().map(|f| (f.spelling, f.inverted)).collect())
                .collect()
        };
        assert_eq!(spellings("°C for 2 h"), [vec![("°C".to_string(), false)]]);
        assert_eq!(
            spellings("mg/kg daily"),
            [
                vec![("mg".to_string(), false)],
                vec![("mg".to_string(), false), ("kg".to_string(), true)]
            ]
        );
        assert_eq!(
            spellings("μg ml⁻¹ of"),
            [
                vec![("μg".to_string(), false)],
                vec![("μg".to_string(), false), ("ml".to_string(), false)]
            ]
        );
        assert_eq!(
            spellings("mg per kg"),
            [
                vec![("mg".to_string(), false)],
                vec![("mg".to_string(), false), ("kg".to_string(), true)]
            ]
        );
        // A bare word after a space ends the expression; a letter or digit after a spelling voids it.
        assert_eq!(spellings("h at 30").len(), 1);
        assert!(scan("BP1", 0).is_empty());
        assert!(scan("′ end", 0).is_empty());
    }

    #[test]
    fn exponents() {
        let e = |s: &str| exponent(s, 0).map(|(e, end)| (e.to_rational(), end));
        assert_eq!(
            e("⁻¹"),
            Some((Rational::parse_decimal("-1").unwrap(), "⁻¹".len()))
        );
        assert_eq!(
            e("²x"),
            Some((Rational::parse_decimal("2").unwrap(), "²".len()))
        );
        assert_eq!(e("^-2"), Some((Rational::parse_decimal("-2").unwrap(), 3)));
        assert_eq!(
            e("^(1/2)"),
            Some((Rational::new(1.into(), 2.into()).unwrap(), 6))
        );
        assert_eq!(e("^0"), None);
        assert_eq!(e("x"), None);
    }
}
