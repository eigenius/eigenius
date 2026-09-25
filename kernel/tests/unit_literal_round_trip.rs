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

//! `compile(print(encode(LitUnit)))` returns the same unit (D93): through D47 encode, the ESL
//! printer, the lexer, the parser, and lowering back to `Exp`.

use eigenius_kernel::esl;
use eigenius_kernel::esl::print::{print_type_expr, Namespaces};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::program::eigentt_type_mirror::{decode_type, encode_type};
use eigenius_kernel::units::Unit;

/// A base unit, a compound with a negative exponent, a rational exponent, and the dimensionless
/// unit.
const CASES: &[&str] = &["m", "s^-2\u{b7}m\u{b7}kg", "m^2/3", "1"];

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
fn a_unit_survives_encode_print_and_reparse() {
    for canonical in CASES {
        let u = Unit::parse_canonical(canonical).expect("canonical");
        let original = Exp::LitUnit(u.clone());
        let encoded =
            encode_type(&original, eigenius_kernel::testing::codec_names()).expect("encodes");

        let tagged = serde_json::json!({"ctor": "LitUnit", "args": [canonical]});
        let mut ns = Namespaces::new();
        let printed = print_type_expr(&tagged, &mut ns).expect("prints");
        assert_eq!(printed.trim(), format!("u\"{canonical}\""));

        assert_eq!(compile_term(&printed), encoded, "round-trip failed for {u}");
    }
}

/// The decoder refuses a non-canonical argument rather than normalising it, as it does for
/// `LitRat`: the encoder only ever emits canonical form, so anything else was authored or corrupted.
#[test]
fn the_decoder_refuses_a_non_canonical_unit() {
    let names = eigenius_kernel::testing::codec_names();
    let good = encode_type(&Exp::LitUnit(Unit::parse_canonical("m").unwrap()), names).unwrap();
    assert!(decode_type(&good, eigenius_kernel::testing::term_chain()).is_ok());

    let bad = eigenius_kernel::testing::term_value(
        &serde_json::json!({"ctor": "LitUnit", "args": ["m\u{b7}m"]}),
    );
    assert!(decode_type(&bad, eigenius_kernel::testing::term_chain()).is_err());
}
