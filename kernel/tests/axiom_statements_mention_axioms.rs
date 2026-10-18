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

//! An axiom's statement may mention another axiom.
//!
//! It could not: `Layer::axiom_env` is a `OnceLock` whose initialiser type-checks every axiom's
//! statement, and checking one that mentions another asked `axiom_env` for its type — re-entering
//! the initialisation and deadlocking, both threads parked in `futex_wait`. Nothing mentioned
//! another axiom until D93's `units:mul` and `units:pow`, whose whole purpose is to appear in other
//! declarations' types. Admission is now on demand within the construction
//! (`program::axiom_env::axiom_type`).
//!
//! Every test runs under a watchdog, so a regression FAILS instead of hanging the suite — the
//! first probe of this ran for 44 minutes before anyone looked.

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::validation::{ValidationRule, Validator};
use std::sync::Arc;
use std::time::Duration;

/// Validate `source` on the bootstrap chain on another thread, failing if it has not finished in
/// two minutes. A bootstrap and validation take seconds.
fn validate_within_deadline(source: &str) -> Vec<(ValidationRule, String)> {
    let source = format!(
        "namespace core = \"urn:eigenius:core\";\nnamespace probe = \"urn:eigenius:probe\";\n{source}\n"
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let storage = LayerStorage::in_memory();
            let ctx = bootstrap_with_storage(storage.clone()).expect("bootstrap builds");
            let resources = eigenius_kernel::esl::compile(&source, ctx.head())
                .unwrap_or_else(|e| panic!("ESL must compile: {e:?}"));
            let mut b = LayerBuilder::new("probe", Some(Arc::clone(ctx.head())));
            for r in resources {
                b.add_resource(r).expect("add_resource");
            }
            let layer = Arc::new(b.build(storage));
            let errs: Vec<_> = Validator::new(Arc::clone(&layer))
                .validate()
                .into_iter()
                .map(|e| (e.rule, e.message))
                .collect();
            let _ = tx.send(errs);
        })
        .expect("spawn");
    rx.recv_timeout(Duration::from_secs(120))
        .expect("validation did not finish in 120 s — the axiom environment deadlocked")
}

#[test]
fn an_axiom_statement_may_mention_another_axiom() {
    let errs = validate_within_deadline(
        "axiom probe:p : core:string -> Prop\naxiom probe:x : probe:p(\"a\") -> Prop",
    );
    assert!(errs.is_empty(), "{errs:#?}");
}

/// Admission is on demand, so the mentioned axiom may come later in the chain's own order — here
/// `probe:a_uses` sorts before `probe:z_base`.
#[test]
fn the_mentioned_axiom_may_come_later() {
    let errs = validate_within_deadline(
        "axiom probe:a_uses : probe:z_base(\"a\") -> Prop\naxiom probe:z_base : core:string -> Prop",
    );
    assert!(errs.is_empty(), "{errs:#?}");
}

/// A declaration's type is not in the scope of the declaration: a cycle is refused, and says so.
#[test]
fn a_cycle_is_refused_and_named() {
    let errs = validate_within_deadline("axiom probe:a : probe:b\naxiom probe:b : probe:a");
    assert!(
        errs.iter()
            .any(|(rule, msg)| *rule == ValidationRule::TermIllTyped
                && msg.contains("is needed by its own statement")),
        "{errs:#?}"
    );
}

/// One malformed axiom no longer empties the environment. `Layer::axiom_env` returned
/// `build_axiom_env(..).unwrap_or_default()`, so a single failure made EVERY axiom reference on the
/// chain fail as "not registered"; its own doc said the failing axiom would be dropped.
#[test]
fn a_malformed_axiom_does_not_take_the_others_with_it() {
    let errs = validate_within_deadline(
        "axiom probe:bad : probe:missing\n\
         axiom probe:good : core:string -> Prop\n\
         axiom probe:uses_good : probe:good(\"a\") -> Prop",
    );
    assert!(
        errs.iter().all(|(_, msg)| !msg.contains("probe:good")),
        "only probe:bad should fail: {errs:#?}"
    );
    assert!(
        errs.iter()
            .any(|(_, msg)| msg.contains("probe:bad") || msg.contains("probe:missing")),
        "probe:bad should fail: {errs:#?}"
    );
}
