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

//! SYN-26-002's chain (D99 step 5), `experiments/pmi-registry/`, on the bootstrap chain: the
//! registry layer, the SYNGENE literature, the case's declarations, its conclusions, and what it
//! refuses.
//!
//! WordNet and HPO are stubbed; `syn_26_002_on_snapshot.rs` runs the same checks against the
//! loaded lexicon.

mod common;

use common::{chain_on, BootstrapBase};

#[test]
fn the_chain_validates() {
    common::check_chain_validates(&chain_on(&BootstrapBase));
}

#[test]
fn claims_6_and_7_ground_nothing_about_the_proband() {
    common::check_refusals(&chain_on(&BootstrapBase));
}

#[test]
fn each_claim_has_its_ground_and_none_is_observed() {
    let chain = chain_on(&BootstrapBase);
    common::check_grounds(&BootstrapBase, &chain);
}
