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

//! `core:short_name` carries `pattern: \S+`, and Rule 5 enforces it against a real chain.
//!
//! The constraint is deliberately weak. It is NOT an identifier pattern: 7638 of the 196941
//! `short_name` values in the tree are NCBI gene symbols where hyphens are standard
//! nomenclature (`NKX3-2`, `H1-0`, `GAS8-AS1`, `TRNAG-GCC`), and eigenius#31 asks to ALLOW
//! hyphens in local names rather than ban them. "No whitespace" is the one property that
//! actually holds across all 196941 values, and it is the one that matters for the external
//! integrations the property exists for.
//!
//! This test exists because a declared constraint that never fires is worse than none — see
//! Rule 4, which was unreachable from the commit path for its whole life (eigenius#118).

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::validation::Validator;
use std::sync::Arc;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Build a one-resource layer on the real bootstrap and validate it.
fn validate_with_short_name(short_name: &str) -> Vec<String> {
    let storage = LayerStorage::in_memory();
    let ctx = bootstrap_with_storage(storage.clone()).expect("bootstrap builds");
    let mut b = LayerBuilder::new("probe", Some(Arc::clone(ctx.head())));
    let mut r = Resource::new(iri("urn:eigenius:probe:thing"));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String(
            iri("urn:eigenius:core:Class").as_str().to_string(),
        )]),
    );
    r.set(
        iri("urn:eigenius:core:description"),
        Value::String("probe".into()),
    );
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String(short_name.into()),
    );
    b.add_resource(r).unwrap();
    let layer = Arc::new(b.build(storage));
    Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| e.message)
        .collect()
}

#[test]
fn whitespace_in_short_name_is_rejected() {
    let errors = validate_with_short_name("Eigenius core team");
    assert!(
        errors.iter().any(|m| m.contains("does not match pattern")),
        "a short_name with a space must be rejected, got: {errors:?}"
    );
}

#[test]
fn hyphenated_gene_symbols_still_validate() {
    // The reason the pattern is `\S+` and not an identifier pattern. If this ever fails,
    // the NCBI gene lexicon stops loading.
    for symbol in ["NKX3-2", "H1-0", "GAS8-AS1", "TRNAG-GCC", "program-run"] {
        let errors = validate_with_short_name(symbol);
        assert!(
            errors.is_empty(),
            "`{symbol}` is a legitimate short_name but was rejected: {errors:?}"
        );
    }
}
