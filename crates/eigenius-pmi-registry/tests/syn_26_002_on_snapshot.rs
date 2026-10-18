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

//! SYN-26-002's chain (D99 step 6) against a snapshot carrying WordNet, UMLS and HPO: the
//! registry's phenotype rows type-check against the real `wn:v00065370_t` and HP classes, and
//! the checks of `syn_26_002_chain.rs` hold.
//!
//!   EIGENIUS_DB_SNAPSHOT=../db-snapshot/uab-d99-r2-hpo-aligned-2026-10-02 \
//!     cargo test --release -p eigenius-pmi-registry --test syn_26_002_on_snapshot -- --ignored --nocapture
//!
//! The snapshot is opened as a working copy; it is not modified.

mod common;

use std::path::PathBuf;

use common::{chain_on, SnapshotBase};
use eigenius_encoding::snapshot::open_head_and_backend;

#[test]
#[ignore = "DB-backed; set EIGENIUS_DB_SNAPSHOT + run --ignored --nocapture"]
fn the_syn_26_002_chain_holds_on_the_lexicon() {
    let Some(snap) = std::env::var("EIGENIUS_DB_SNAPSHOT")
        .ok()
        .map(PathBuf::from)
    else {
        eprintln!("SKIP: EIGENIUS_DB_SNAPSHOT unset");
        return;
    };
    let (head, backend) = match open_head_and_backend(&snap) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return;
        }
    };
    let base = SnapshotBase { head, backend };
    let chain = chain_on(&base);
    common::check_chain_validates(&chain);
    common::check_refusals(&chain);
    common::check_grounds(&base, &chain);
}
