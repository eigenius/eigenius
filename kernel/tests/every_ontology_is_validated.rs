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

//! Every ontology in the tree is validated, not just the ones in `BOOTSTRAP_CHAIN`
//! (eigenius#234).
//!
//! **The rules were never the gap.** Rule 14 already checks that every IRI in a class's
//! `requires` / `recommends` / `subclass_of`, and in a property's `class_types` /
//! `data_type`, resolves to a declaration of the right kind; Rule 1 already checks that
//! an instance carries what its class requires. What was missing is that 24 of the 44
//! ontology files in the tree are outside the bootstrap chain, and nothing put them
//! through a validating path — so the rules never saw them.
//!
//! **This covers 23 of those 24.** The glob is `ontologies/*.json`. The one out-of-chain
//! ESL document, `encoding/claim-kind-alignment.esl`, is excluded because it cannot be
//! validated this way: it redeclares kind classes with `wn:` / `umlscui:` parents that
//! exist only after a lexicon import, so it is snapshot-coupled and reaches a validator
//! through `build-alignment-snapshot.sh` and the encoding tests instead.
//!
//! The issue was filed from a `requires` clause naming a property that P5 had deleted.
//! It survived because the one test that ran the `Validator` over that ontology
//! validated an INSTANCE, and validating an instance does not re-validate the class
//! declaration sitting in a layer below. Both that ontology and that test have since
//! been deleted; this test is the gap they escaped through.
//!
//! **What it found on the first run**, which is the argument for its existence:
//!
//! - `schema-org.eigon.json` carried 2,114 resources attributed to `urn:schema_org`,
//!   which no layer declared. `convert.rs::emit_declarer` mints that `prov:Organization`
//!   and has since D72 — but the checked-in file is a build artifact that was last
//!   edited by a one-line rename sweep instead of regenerated, so it predates its own
//!   generator. Regenerating produced exactly the same 2,114 resources plus the
//!   declarer.
//! - `dock-assay.json` had three properties with no `core:description` and a
//!   `program:Component` missing all three of its required slots.
//!
//! **Why the files load together rather than one at a time.** An `examples.json`
//! instantiates what its sibling `ontology.json` declares, a `registration.json` names
//! those classes, and `comorphisms.json` reaches across two institution directories.
//! Loading each alone reports its dependencies as unresolved, which says nothing about
//! the file. Insertion ORDER within the combined layer does not matter: `add_resource`
//! is a map insert and the validator resolves against the finished layer, so order could
//! only matter for a duplicate `@id`, and the 23 documents share none.

use std::collections::BTreeSet;
use std::sync::Arc;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kernel/ has a parent")
        .to_path_buf()
}

/// Every tracked ontology document, as repo-relative paths.
fn ontology_files(root: &std::path::Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "ontologies/*.json"])
        .output()
        .expect("git ls-files runs");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8(out.stdout)
        .expect("paths are utf-8")
        .lines()
        .map(str::to_string)
        .collect()
}

/// The ontology documents `BOOTSTRAP_CHAIN` is built from, as repo-relative paths.
///
/// Read from the `const BOOTSTRAP_CHAIN` block alone, and only from its `include_str!`
/// arguments. A substring search over the whole module would also match a path named in
/// a comment or a `#[cfg(test)]` fixture — and this module has a comment whose entire
/// purpose is to say a file is NOT in the chain. Matching that would have silently
/// removed a file from coverage with no signal.
fn bootstrap_chain_sources(bootstrap_src: &str) -> BTreeSet<String> {
    let start = bootstrap_src
        .find("const BOOTSTRAP_CHAIN")
        .expect("bootstrap/mod.rs declares BOOTSTRAP_CHAIN");
    let block = &bootstrap_src[start..];
    let end = block
        .find("\n];")
        .expect("the BOOTSTRAP_CHAIN array is terminated");
    block[..end]
        .match_indices("include_str!(\"")
        .filter_map(|(i, m)| {
            let rest = &block[i + m.len()..];
            rest.find('"').map(|q| rest[..q].to_string())
        })
        // `include_str!` paths are relative to kernel/src/bootstrap/.
        .map(|p| p.trim_start_matches("../../../").to_string())
        .collect()
}

#[test]
fn every_json_ontology_outside_the_bootstrap_chain_still_validates() {
    let root = repo_root();
    // `BOOTSTRAP_CHAIN` names its layers with `include_str!`, so the module source is
    // the list. Reading it keeps this test honest when a layer joins or leaves the
    // chain: a file that moves in stops being checked here and starts being checked by
    // the bootstrap itself.
    let bootstrap_src = std::fs::read_to_string(root.join("kernel/src/bootstrap/mod.rs"))
        .expect("bootstrap module is readable");
    // `BOOTSTRAP_CHAIN` is private, so its membership is read from the source. That keeps
    // this test honest when a layer joins or leaves: a file that moves in stops being
    // checked here and starts being checked by the bootstrap itself.

    let in_chain = bootstrap_chain_sources(&bootstrap_src);
    assert!(
        in_chain.len() > 10,
        "the BOOTSTRAP_CHAIN scrape found only {} sources, which means it stopped working \
         rather than that the chain shrank",
        in_chain.len()
    );
    let out_of_chain: Vec<String> = ontology_files(&root)
        .into_iter()
        .filter(|f| !in_chain.contains(f))
        .collect();
    assert!(
        !out_of_chain.is_empty(),
        "no ontology sits outside the bootstrap chain — if that is now true, delete this test"
    );

    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let mut builder =
        eigenius_kernel::layer::LayerBuilder::new("out-of-chain", Some(Arc::clone(ctx.head())));
    for f in &out_of_chain {
        let text = std::fs::read_to_string(root.join(f)).expect("ontology is readable");
        let resources = eigenius_kernel::ontology::eigon_json::parse_document(&text)
            .unwrap_or_else(|e| panic!("{f} does not parse as Eigon-JSON: {e}"));
        for r in resources {
            builder
                .add_resource(r)
                .unwrap_or_else(|e| panic!("{f}: adding a resource failed: {e}"));
        }
    }
    let layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));

    let errors = eigenius_kernel::validation::Validator::new(layer).validate();
    assert!(
        errors.is_empty(),
        "{} validation error(s) across {} ontologies outside the bootstrap chain:\n{}",
        errors.len(),
        out_of_chain.len(),
        errors
            .iter()
            .take(20)
            .map(|e| format!("  [{:?}] {:?} {}", e.rule, e.resource_id, e.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
