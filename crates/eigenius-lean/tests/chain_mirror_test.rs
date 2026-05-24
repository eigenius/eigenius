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

//! Phase 20a.4 acceptance: the chain-mirror translator round-trips
//! a hand-vendored export's target type into a `lean:LeanExpr`
//! tagged-dict value with stable structure and deterministic CBOR
//! encoding (per D40 §7).
//!
//! Uses the same `toy_proof_holds.json` fixture as
//! `toy_check_test`. That file's `PUnit` inductive declaration is
//! the smallest hand-built closed Lean environment we vendor; its
//! type is `Sort u` (one universe-polymorphic parameter `u`), which
//! exercises four of the `lean:LeanExpr` ctor paths in a single
//! mirror: `Expr.Sort`, `Level.Param`, `Name.Str`, and `Name.Anon`.

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use eigenius_lean::{bytes_to_lean_expr, ChainMirrorError};

const TOY_HOLDS: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");

#[test]
fn translate_punit_type_emits_sort_param() {
    // PUnit's type in the vendored export is `Sort u` — universe
    // parameter `u`. The chain mirror should be a `Sort` node
    // wrapping a `Param "u"` level wrapping a name built up from
    // `Anon`.
    let value = bytes_to_lean_expr(TOY_HOLDS, "PUnit").expect("translation must succeed");
    let json = match &value {
        Value::Json(j) => j,
        other => panic!("expected Value::Json, got {other:?}"),
    };

    let expected = serde_json::json!({
        "ctor": "Sort",
        "args": [
            {
                "ctor": "Param",
                "args": [
                    {
                        "ctor": "Str",
                        "args": [
                            {"ctor": "Anon"},
                            "u"
                        ]
                    }
                ]
            }
        ]
    });

    assert_eq!(
        json, &expected,
        "PUnit's type must mirror to `Sort (Param (Str Anon \"u\"))`"
    );
}

#[test]
fn translation_is_deterministic_under_cbor_serialisation() {
    // D40 §7's determinism contract: same input bytes → same chain
    // CBOR. We assert byte-equality on the canonicalised CBOR
    // encoding of a Resource that carries the mirrored value as a
    // property. Repeating the translation must not surface any
    // allocation-order or hash-cons-identity nondeterminism.
    let first = bytes_to_lean_expr(TOY_HOLDS, "PUnit").expect("first translation");
    let second = bytes_to_lean_expr(TOY_HOLDS, "PUnit").expect("second translation");

    let first_bytes = canonical_cbor_for(&first);
    let second_bytes = canonical_cbor_for(&second);
    assert_eq!(
        first_bytes, second_bytes,
        "two translations of the same export must produce byte-equal CBOR"
    );

    // And the value isn't trivially empty — sanity-check we encoded
    // something substantive.
    assert!(
        first_bytes.len() > 8,
        "deterministic CBOR encoding must be non-trivial; got {} bytes",
        first_bytes.len()
    );
}

#[test]
fn missing_target_surfaces_target_not_found() {
    let err =
        bytes_to_lean_expr(TOY_HOLDS, "DoesNotExist").expect_err("must error on missing target");
    match err {
        ChainMirrorError::TargetNotFound(name) => {
            assert_eq!(
                name, "DoesNotExist",
                "diagnostic should carry the missing name"
            );
        }
        other => panic!("expected TargetNotFound, got {other:?}"),
    }
}

#[test]
fn malformed_bytes_surface_parse_failed() {
    let err = bytes_to_lean_expr(b"this is not a valid export\n", "x")
        .expect_err("must error on malformed export");
    assert!(
        matches!(err, ChainMirrorError::ParseFailed(_)),
        "expected ParseFailed, got {err:?}"
    );
}

/// Wrap a mirrored value as the sole property of an embedded
/// Resource and serialise via the kernel's deterministic CBOR
/// encoder. The wrapper is a stand-in for the `proposition`
/// property on a real `LeanProofTerm`; the discipline we want to
/// assert is that the inner `Value::Json` encoding is stable across
/// calls.
fn canonical_cbor_for(value: &Value) -> Vec<u8> {
    let mut r = Resource::new_embedded();
    let prop_iri = Iri::parse("urn:eigenius:lean:proposition").expect("static IRI");
    r.set(prop_iri, value.clone());
    eigon_cbor::serialize_resource(&r)
}
