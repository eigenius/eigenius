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

//! `compile(print(encode(LitRat)))` returns the same rational (D94).
//!
//! The values are the ones that motivated exact numerics, so this is not a shape test: each is a
//! number a JSON number cannot carry, and each survives the whole surface — D47 encode, the ESL
//! printer, the lexer, the parser, and lowering back to `Exp`.

use eigenius_kernel::esl;
use eigenius_kernel::esl::print::{print_type_expr, Namespaces};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::numeric::Rational;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;
use num_bigint::BigInt;
use num_traits::Pow;

fn cases() -> Vec<(Rational, &'static str)> {
    vec![
        (Rational::new(1.into(), 20.into()).unwrap(), "1/20"),
        // Degrees to radians (D93). No terminating decimal, which is why the canonical form
        // exists alongside the `0.05r` suffix.
        (Rational::new(37.into(), 180.into()).unwrap(), "37/180"),
        (Rational::new((-1).into(), 3.into()).unwrap(), "-1/3"),
        (Rational::from_integer(7.into()).unwrap(), "7"),
        // The exact binary64 0.05: a 57-bit denominator, past core:integer's 53-bit safe range.
        (
            Rational::new(
                BigInt::from(3602879701896397i64),
                BigInt::from(2).pow(56u32),
            )
            .unwrap(),
            "3602879701896397/72057594037927936",
        ),
        // 1 eV = 1.602176634e-19 J. Authored as 1602176634/10^28; the canonical form divides
        // both by 2, which is the reduction the constructor performs and the lexer insists on.
        (
            Rational::new(BigInt::from(1602176634i64), BigInt::from(10).pow(28u32)).unwrap(),
            "801088317/5000000000000000000000000000",
        ),
    ]
}

/// Compile one `type_expr(...)` body against the bootstrap chain and hand back the term.
fn compile_term(body: &str) -> eigenius_kernel::ontology::resource::Value {
    let layer = eigenius_kernel::testing::term_chain();
    let src = format!(
        "namespace rt = \"urn:eigenius:roundtrip\";\n\n\
         resource rt:probe : rt:Probe {{\n    rt:term = type_expr( {body} );\n}}\n"
    );
    let resources = esl::compile(&src, layer).unwrap_or_else(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        panic!("compile failed: {}\n--- source ---\n{src}", msgs.join("; "))
    });
    let iri = Iri::parse("urn:eigenius:roundtrip:term").expect("well-formed IRI");
    for r in &resources {
        if let Some(v) = r.get(&iri) {
            return v.clone();
        }
    }
    panic!("no rt:term in compiled output");
}

#[test]
fn a_rational_survives_encode_print_and_reparse() {
    for (r, canonical) in cases() {
        let original = Exp::LitRat(r.clone());
        let encoded =
            encode_type(&original, eigenius_kernel::testing::codec_names()).expect("encodes");

        // The printer consumes the TAGGED form; the encoder produces the resource form. Both
        // describe one term, and the test closes the loop by compiling back to the resource form.
        let tagged = serde_json::json!({"ctor": "LitRat", "args": [canonical]});
        let mut ns = Namespaces::new();
        let printed = print_type_expr(&tagged, &mut ns).expect("prints");
        // The printer emits the canonical form, which always reparses — a decimal could not
        // express 37/180 at all.
        assert_eq!(printed.trim(), format!("r\"{canonical}\""));

        assert_eq!(compile_term(&printed), encoded, "round-trip failed for {r}");
    }
}

#[test]
fn the_decimal_suffix_and_the_canonical_form_agree() {
    // `0.05r` and `r"1/20"` are two spellings the surface admits for one value. A bare `0.05`
    // is a DIFFERENT number — the binary64 — which is the distinction the suffix exists to make.
    let from_suffix = compile_term("0.05r");
    let from_canonical = compile_term("r\"1/20\"");
    assert_eq!(from_suffix, from_canonical);
    assert_ne!(from_suffix, compile_term("0.05"));
}
