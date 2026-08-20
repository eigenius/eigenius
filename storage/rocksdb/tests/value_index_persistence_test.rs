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

//! D65 slices 0+2 over the **real RocksStore** — the exact backend the deployed
//! service uses (`serve --db` → `bootstrap_persistent`).
//!
//! Two facts, both live assertions:
//!
//! - [`form_value_index_populates_against_rocksdb_at_seed`] — at SEED the
//!   `lexicon:form` ValueIndex is active, the closed-class determiner forms are
//!   value-indexed in the (in-memory-at-seed) index, and a `LexicalIndex` takes
//!   the lazy path. This is the genuine slice-0+2 witness on the production stack.
//!
//! - [`derived_indexes_survive_resume`] — seeds a chain, reopens the same
//!   directory, and asserts that the triple index and the `lexicon:form` value
//!   index are both still populated. This test was `#[ignore]`d when it was
//!   written, because it recorded a real structural gap: SEED built the
//!   bootstrap chain on *in-memory* storage and persisted each layer with
//!   resources, topology and bloom only, so a resumed service saw empty derived
//!   indexes for every seeded layer. The gap is closed —
//!   `RocksStore::store_layer` calls `populate_layer_indexes` on the
//!   post-validation persist path every commit funnels through, so a seeded
//!   layer's index entries are durable — and the `#[ignore]` is gone. Do not
//!   restore it: this test now guards the fix.

use std::sync::Arc;

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::dcg::LexicalIndex;
use eigenius_kernel::layer::{
    normalize_value, resolve_active_value_indexes, scan_chain, ActiveValueIndex, Layer,
};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;

/// The active `lexicon:form` ValueIndex at `head`, or `None`.
fn form_index(head: &Arc<Layer>) -> Option<ActiveValueIndex> {
    let form_prop = Iri::parse("urn:eigenius:lexicon:form").unwrap();
    resolve_active_value_indexes(head)
        .into_iter()
        .find(|a| a.target_property == form_prop)
}

/// Resolve a surface form against the head's value index — the subjects, with the
/// key normalised through the index's declared normalizer.
fn lookup_form(head: &Arc<Layer>, idx: &ActiveValueIndex, surface: &str) -> Vec<Iri> {
    let key = normalize_value(&idx.normalizer, surface);
    head.storage()
        .value_index
        .lookup(&idx.iri, &key)
        .map(|r| r.unwrap().0)
        .collect()
}

#[test]
fn form_value_index_populates_against_rocksdb_at_seed() {
    let dir = TempDir::new().unwrap();
    let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
    let ctx = bootstrap_persistent(Arc::clone(&backend)).expect("seed bootstrap");
    let head = ctx.head();

    let idx = form_index(head).expect("the lexicon:form ValueIndex is active at seed");
    assert_eq!(
        idx.normalizer.as_str(),
        "urn:eigenius:core:normalizers:lowercase"
    );

    // The closed-class determiner forms were value-indexed at seed, and resolve
    // case-insensitively through the lowercase normalizer.
    assert!(
        !lookup_form(head, &idx, "every").is_empty(),
        "'every' must be value-indexed at seed"
    );
    assert!(
        !lookup_form(head, &idx, "EVERY").is_empty(),
        "lookup is case-insensitive via the lowercase normalizer"
    );
    assert!(
        !lookup_form(head, &idx, "is").is_empty(),
        "the copula 'is' must be value-indexed at seed"
    );
    assert!(
        lookup_form(head, &idx, "notaword").is_empty(),
        "an absent form resolves to nothing"
    );

    // A LexicalIndex over the head takes the LAZY path (RocksStore ⇒ shared index,
    // form index active): O(1) build, nothing cached until a parse probes.
    let lex = LexicalIndex::build(Arc::clone(head));
    assert_eq!(
        lex.len(),
        0,
        "the lazy LexicalIndex caches no forms before any parse"
    );
}

#[test]
fn derived_indexes_survive_resume() {
    let dir = TempDir::new().unwrap();
    let is_a = Iri::parse("urn:eigenius:core:is_a").unwrap();
    let class = Iri::parse("urn:eigenius:core:Class").unwrap();

    // SEED.
    {
        let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
        let ctx = bootstrap_persistent(Arc::clone(&backend)).expect("seed");
        assert!(
            !scan_chain(ctx.head(), &is_a, &class).is_empty(),
            "triple index is populated at seed"
        );
        let idx = form_index(ctx.head()).expect("form index active at seed");
        assert!(!lookup_form(ctx.head(), &idx, "every").is_empty());
    }

    // RESUME (reopen the same dir).
    {
        let backend: Arc<dyn PersistentBackend> = Arc::new(RocksStore::open(dir.path()).unwrap());
        let ctx = bootstrap_persistent(Arc::clone(&backend)).expect("resume");
        // These are the assertions the fix must make pass.
        assert!(
            !scan_chain(ctx.head(), &is_a, &class).is_empty(),
            "triple index must survive resume"
        );
        let idx = form_index(ctx.head()).expect("form index must be discoverable after resume");
        assert!(
            !lookup_form(ctx.head(), &idx, "every").is_empty(),
            "value index must survive resume"
        );
    }
}
