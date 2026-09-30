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

//! D95 slice 3 — quantity tokens, read against the chain's own units layer and unit spellings, on
//! the inventory D95 draws from the WRN methods.

use std::sync::{Arc, OnceLock};

use eigenius_kernel::dcg::{tokenize, Identity, Parser, ProseUnits, Token, TokenKind};
use eigenius_kernel::numeric::Rational;

fn units() -> &'static ProseUnits {
    static U: OnceLock<ProseUnits> = OnceLock::new();
    U.get_or_init(|| ProseUnits::load(eigenius_kernel::testing::bootstrap_context().head()))
}

fn q(n: i64, d: i64) -> Rational {
    Rational::new(n.into(), d.into()).unwrap()
}

/// A reading as (stated, coefficient, unit).
type Reading = (String, Rational, String);

/// Each token's surface, and for a quantity its readings.
fn read(text: &str) -> Vec<(String, Vec<Reading>)> {
    tokenize(text, units())
        .iter()
        .map(|t: &Token| {
            let readings = match t.kind() {
                TokenKind::Quantity(q) => q
                    .readings
                    .iter()
                    .map(|r| {
                        (
                            r.stated.clone(),
                            r.value.magnitude.coefficient().clone(),
                            r.value.unit.to_canonical_string(),
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            };
            (t.surface().to_string(), readings)
        })
        .collect()
}

/// The one quantity in `text`: its surface and readings.
fn the_quantity(text: &str) -> (String, Vec<Reading>) {
    let mut qs: Vec<_> = read(text)
        .into_iter()
        .filter(|(_, r)| !r.is_empty())
        .collect();
    assert_eq!(qs.len(), 1, "{text:?}: {qs:?}");
    qs.pop().unwrap()
}

fn one(stated: &str, coefficient: Rational, unit: &str) -> Vec<Reading> {
    vec![(stated.to_string(), coefficient, unit.to_string())]
}

/// The WRN sentence D95 opens with: `931g` has two readings, and `2 h` and `30 °C` one each.
#[test]
fn the_centrifugation_sentence() {
    let tokens = read("the plates were spun at 931g for 2 h at 30 °C");
    let surfaces: Vec<&str> = tokens.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(
        surfaces,
        ["the", "plates", "were", "spun", "at", "931g", "for", "2 h", "at", "30 °C"]
    );
    assert_eq!(
        tokens[5].1,
        vec![
            ("g".to_string(), q(931, 1000), "kg".to_string()),
            (
                "g_n".to_string(),
                q(182_599_823, 20_000),
                "s^-2·m".to_string()
            ),
        ]
    );
    assert_eq!(tokens[7].1, one("h", q(7200, 1), "s"));
    assert_eq!(tokens[9].1, one("°C", q(6063, 20), "K"));
    // Digit groups before an attached unit.
    assert_eq!(
        the_quantity("centrifuged at 1,000g"),
        (
            "1,000g".to_string(),
            vec![
                ("g".to_string(), q(1, 1), "kg".to_string()),
                ("g_n".to_string(), q(196_133, 20), "s^-2·m".to_string()),
            ]
        )
    );
}

#[test]
fn the_methods_inventory() {
    for (text, surface, stated, coefficient, unit) in [
        (
            "10 μg ml⁻¹ of gentamicin",
            "10 μg ml⁻¹",
            "μg·mL^-1",
            q(1, 100),
            "m^-3·kg",
        ),
        (
            "a dose of 5 mg/kg",
            "5 mg/kg",
            "mg·kg^-1",
            q(1, 200_000),
            "1",
        ),
        (
            "5 mg per kg daily",
            "5 mg per kg",
            "mg·kg^-1",
            q(1, 200_000),
            "1",
        ),
        ("in 5 mM EDTA", "5 mM", "mM", q(5, 1), "m^-3·mol"),
        ("10 µM etoposide", "10 µM", "μM", q(1, 100), "m^-3·mol"),
        ("for 9 days", "9 days", "d", q(777_600, 1), "s"),
        ("after 2 weeks", "2 weeks", "wk", q(1_209_600, 1), "s"),
        ("with 10% FBS", "10%", "1", q(1, 10), "1"),
        (
            "in 0.2 ml of medium",
            "0.2 ml",
            "mL",
            q(1, 5_000_000),
            "m^3",
        ),
        ("at 37 ºC", "37 ºC", "°C", q(6203, 20), "K"),
        ("stored at −80 °C", "−80 °C", "°C", q(3863, 20), "K"),
        (
            "spun at 931 RCF",
            "931 RCF",
            "g_n",
            q(182_599_823, 20_000),
            "s^-2·m",
        ),
        (
            "heated at 2 °C/min",
            "2 °C/min",
            "°C·min^-1",
            q(1, 30),
            "s^-1·K",
        ),
    ] {
        let (got_surface, readings) = the_quantity(text);
        assert_eq!(got_surface, surface, "{text:?}");
        assert_eq!(readings, one(stated, coefficient, unit), "{text:?}");
    }
}

/// What does not read as a quantity.
#[test]
fn what_is_not_a_quantity() {
    for text in [
        "53BP1 foci",
        "HEK293T cells",
        "a 5-fold change",
        "the 3′ end",
        "the 5′-UTR",
        "12 cells per well",
        "chromosomes 3 and 5",
        "96-well plates",
    ] {
        assert!(
            read(text).iter().all(|(_, r)| r.is_empty()),
            "{text:?}: {:?}",
            read(text)
        );
    }
    // A bare word after a unit is not a second factor: `h at` is not hour·attotonne.
    let tokens = read("incubated for 2 h at 37 °C");
    let surfaces: Vec<&str> = tokens.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(surfaces, ["incubated", "for", "2 h", "at", "37 °C"]);
}

/// Slice 6b: scientific notation is one numeral — a mantissa, `×` or `x`, and a power of ten, or a
/// power of ten alone with a superscript or caret. Extracted text writes the exponent with a plain
/// minus (`10-16`), which after `×` can only be an exponent.
#[test]
fn scientific_notation_is_one_numeral() {
    for (text, surface, value) in [
        ("P < 2 × 10⁻¹⁶", "2 × 10⁻¹⁶", q(1, 5_000_000_000_000_000)),
        ("P <2.2× 10-16", "2.2× 10-16", q(11, 50_000_000_000_000_000)),
        (
            "P = 4.2 × 10^-13",
            "4.2 × 10^-13",
            q(21, 50_000_000_000_000),
        ),
        ("1.5 x 10³ cells per well", "1.5 x 10³", q(1500, 1)),
        ("seeded at 10⁶ cells", "10⁶", q(1_000_000, 1)),
    ] {
        let tokens = tokenize(text, units());
        let numeral = tokens
            .iter()
            .find(|t| matches!(t.kind(), TokenKind::Numeral(_)))
            .unwrap_or_else(|| panic!("{text:?}: no numeral in {tokens:?}"));
        assert_eq!(numeral.surface(), surface, "{text:?}");
        assert_eq!(numeral.kind(), &TokenKind::Numeral(value), "{text:?}");
    }
    // Without `×`, `10-16` is not a power: it could be a range or a catalogue number.
    let tokens = tokenize("10-16 cells", units());
    assert!(tokens[0].is_word(), "{tokens:?}");
}

/// Slice 6c: a digit pair with a unit or `%` after it, joined by an en-dash or a hyphen, is one range
/// token, both endpoints read in that unit — `30–37 °C` is 303.15 K to 310.15 K. An en-dash pair with no
/// unit is a count range, read at the dimensionless unit (slice 7); a hyphen pair with no unit is not a
/// range, since that is how a catalogue number is written.
#[test]
fn a_range_is_one_token() {
    for (text, surface, low, high, unit) in [
        (
            "every 2–3 days",
            "2–3 days",
            q(172_800, 1),
            q(259_200, 1),
            "s",
        ),
        (
            "every 2-3 days",
            "2-3 days",
            q(172_800, 1),
            q(259_200, 1),
            "s",
        ),
        (
            "reached 80–90% confluence",
            "80–90%",
            q(4, 5),
            q(9, 10),
            "1",
        ),
        ("45-60% of such cancers", "45-60%", q(9, 20), q(3, 5), "1"),
        ("in 4–12% gels", "4–12%", q(1, 25), q(3, 25), "1"),
        ("at 30–37 °C", "30–37 °C", q(6063, 20), q(6203, 20), "K"),
    ] {
        let tokens = tokenize(text, units());
        let range = tokens
            .iter()
            .find_map(|t| match t.kind() {
                TokenKind::Range(r) => Some((t.surface().to_string(), r.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?}: no range in {tokens:?}"));
        assert_eq!(range.0, surface, "{text:?}");
        let (lo, hi) = (
            &range.1.low.readings[0].value,
            &range.1.high.readings[0].value,
        );
        assert_eq!(
            (lo.magnitude.coefficient(), hi.magnitude.coefficient()),
            (&low, &high),
            "{text:?}"
        );
        assert_eq!(lo.unit.to_canonical_string(), unit, "{text:?}");
        assert_eq!(hi.unit, lo.unit, "{text:?}");
    }
    let count = tokenize("4–7 foci", units());
    match count[0].kind() {
        TokenKind::Range(r) => {
            assert!(r.unitless, "{count:?}");
            assert_eq!((&r.low.value, &r.high.value), (&q(4, 1), &q(7, 1)));
            assert!(r.low.readings[0].value.unit.is_dimensionless());
        }
        other => panic!("`4–7 foci`: {other:?}"),
    }
    for text in [
        "catalogue number 926-68021",
        "96-well plates",
        "a 5-fold change",
    ] {
        assert!(
            tokenize(text, units())
                .iter()
                .all(|t| !matches!(t.kind(), TokenKind::Range(_))),
            "{text:?}"
        );
    }
}

/// Slice 7d, decisions 11 and 12: a number word is a numeral, in any case, so a unit after it makes a
/// quantity (`Nine days` is 777600 s); a numeral joined by a hyphen to a unit name is a quantity too.
/// A hyphen before a unit SYMBOL names a compound, and a unit that does not end the token (`8-week-old`)
/// is not read.
#[test]
fn a_number_word_is_a_numeral() {
    for (text, surface, stated, value) in [
        (
            "Nine days after doxycycline treatment",
            "Nine days",
            "d",
            q(777_600, 1),
        ),
        (
            "recovered by three weeks",
            "three weeks",
            "wk",
            q(1_814_400, 1),
        ),
        (
            "Seven days post-transduction",
            "Seven days",
            "d",
            q(604_800, 1),
        ),
        (
            "with an eight-day viability assay",
            "eight-day",
            "d",
            q(691_200, 1),
        ),
        ("with an 8-day viability assay", "8-day", "d", q(691_200, 1)),
        (
            "in a seven-day viability assay",
            "seven-day",
            "d",
            q(604_800, 1),
        ),
        ("a 10-minute incubation", "10-minute", "min", q(600, 1)),
    ] {
        assert_eq!(
            the_quantity(text),
            (surface.to_string(), one(stated, value, "s")),
            "{text:?}"
        );
    }
    for (text, value) in [("three sgRNAs", 3), ("One gene", 1), ("TEN cells", 10)] {
        let tokens = tokenize(text, units());
        assert_eq!(
            tokens[0].kind(),
            &TokenKind::Numeral(q(value, 1)),
            "{text:?}: {tokens:?}"
        );
    }
    for text in [
        "8-week-old mice",
        "treated with 5-mC",
        "treated with 3-MA",
        "treated with 6-TG",
        "a 2-h incubation",
        "one-sided test",
        "someone",
    ] {
        assert!(
            read(text).iter().all(|(_, r)| r.is_empty()),
            "{text:?}: {:?}",
            read(text)
        );
    }
}

/// Decision 4, deferred to D96: in plain text an unbracketed figure panel reads as a quantity.
#[test]
fn an_unbracketed_figure_panel_reads_as_a_quantity() {
    assert_eq!(
        the_quantity("as shown in Fig. 2d"),
        ("2d".to_string(), one("d", q(172_800, 1), "s"))
    );
}

/// The parser tokenizes with the chain's vocabulary, and a quantity token seeds its own items
/// (slice 4), so it is never unseedable.
#[test]
fn the_parser_reads_quantities_and_they_seed() {
    let ctx = eigenius_kernel::testing::bootstrap_context();
    let parser = Parser::build(Arc::clone(ctx.head()));
    let tokens = parser.tokenize("incubated at 37 °C");
    assert!(matches!(tokens[2].kind(), TokenKind::Quantity(_)));
    assert!(parser
        .unseedable_tokens("incubated at 37 °C", &Identity)
        .is_empty());
}

/// Every quantity the recogniser reads in the WRN texts, with its readings, and a count by stated
/// unit. Run with `--ignored --nocapture`; it reads the gitignored texts under `references/`.
#[test]
#[ignore = "reads the gitignored WRN texts under references/"]
fn list_the_quantities_in_the_wrn_texts() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../references/publications/WRN-Helicase-Nature-OCR"
    );
    let mut by_unit: std::collections::BTreeMap<String, usize> = Default::default();
    for name in ["methods.txt", "letter-body.txt"] {
        let path = format!("{dir}/{name}");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut n = 0;
        for s in eigenius_kernel::dcg::segment_sentences(&text) {
            for (surface, readings) in read(&s) {
                if readings.is_empty() {
                    continue;
                }
                n += 1;
                let shown: Vec<String> = readings
                    .iter()
                    .map(|(stated, c, u)| format!("{stated} → {} {u}", c.to_canonical_string()))
                    .collect();
                println!("{name}: {surface:<16} {}", shown.join(" | "));
                for (stated, _, _) in &readings {
                    *by_unit.entry(stated.clone()).or_default() += 1;
                }
            }
        }
        println!("{name}: {n} quantities\n");
    }
    println!("by stated unit: {by_unit:?}");
}
