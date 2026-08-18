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

//! **The bootstrap manifest is PINNED.**
//!
//! Editing any embedded ontology changes [`eigenius_kernel::bootstrap::current_manifest`], and every
//! already-persisted store then refuses to resume with `BootstrapError::ManifestDrift`. This test
//! makes that consequence surface HERE — in `cargo test`, with no database and no snapshot — instead
//! of hours later in whatever first tries to open a store.
//!
//! WHY IT EXISTS (`2026-08-17`). D70 added one value to `lexicon:Num`. That bumped the `lexicon`
//! hash, which invalidated every snapshot on disk, which broke both demos and the D67 §3.5 acceptance
//! artifacts — and `cargo test --workspace` stayed GREEN throughout, because everything that touches a
//! snapshot is `#[ignore]`d and DB-backed. The breakage was found by hand, days later. Nothing cheap
//! was watching the one signal that predicts it.
//!
//! WHY IT IS PINNED HERE RATHER THAN CHECKED AGAINST A SNAPSHOT. A "does the snapshot still resume"
//! test was the first idea and is the wrong instrument: it needs a store on disk, so it would SKIP in
//! CI — precisely where the signal is wanted. The manifest is computed from source alone, so this runs
//! everywhere, and it is the same value the drift check compares, so it fires on exactly the condition
//! that invalidates stores.
//!
//! WHAT IT DOES NOT COVER. A snapshot also goes stale when the LEXICON content changes with no
//! bootstrap edit — the 2026-08-15 atom overrides dropped `drug target` and added `synthetic lethal`,
//! no manifest moved, and recorded draws still missed. Catching that needs a recorded fingerprint of
//! the lexicon an experiment's draws were made against; the reseed's `PROVENANCE` stamp is half of it.

use eigenius_kernel::bootstrap::current_manifest;

/// The manifest as committed. Update it in the SAME commit as any bootstrap ontology edit — see the
/// panic message for the rest of the follow-through.
const EXPECTED: &str = "\
core:3f93984ab069b6fa6b030f2aadca32051259424817f1eb82a8bd39e22f43c0f9
eigentt-type-fragment:4fb2c97c14e5ef00e1b8e1c842563b2150d19b1cfcf2d8ed245c762a742e057c
program:23d0359ea194547b9dd81c3672d0290278a8db6f171637a273e3efb41c515b1a
reflection:133a9eef8ae96d7a976ead80e11deb3f3786a9b73e43e5b2e3a6cf5146af025a
obo:8d83d77e0935dbb15f359a9fbfce7aceb681e761aeeac40f06e75e16afe9f784
institution:27871f87612484d6469b66a4b0379731152af0c1f8eb04007c8b0343f4648c13
runtime:4c05dc3b114acb2554e8f8d594a6878f94e8f60f3a512d46eb816d3a030f26cc
formulas:ba63b387d496f46effd86b6e544c2daebea69b605c5b366123f17317bbae7957
lean-expressions:6263f64c4fb167dedb9ba69c2e353517bd343b21da7ad6aa346f42f2b975fac5
lean-runtime-classes:11de512ae4aea72e0865a19becdefd4daed9c9f6cdf2abd5af3b88d17078294e
lean-institution:e48be69b9df06f02232feac048fc4ec5bdebcf62b8a50b90145b9f62176610dc
reasoning:63e212e4ae4254941d57616e8497ef2fdf025c84bcf6461e07006762011ded58
statistics:0e179b5ef88c9e01399a84d6c863b6ddb4fe38859374c45b3f685175d584af3b
notebook:2f0945ef2e6cbd5c7f224e5b286938667eef36546605509b606aa86115bbc6c2
ingest:67534b5bcf3478a18bd5df2a3c856132e8702dd3f9d3e8727b40179794ae0aba
reference:685e1bd6fbd0a6f4285d674eb1ccac459835c788cde49d4125fde076ed002d6a
logic:bde155e5644cb03e236cd94a501301b878832707d0ba9c6a361fa204ce9e813d
lexicon:51aa4972dd065159a0124ed22e776819a9016d92a4fdfb4d8841ad56581e81fa
ontology:12321a1bb48ad9f89071cd6500a0791f4a33a60ced9c4fdaf0e82e6dae9faa70
closed-class:7c15f2350e7de9e4e1af0291f2cedc2753867265841d09c3be7192dc7f91813d
";

/// Per-layer diff, so the failure says WHICH ontology moved rather than only that something did. On
/// the D70 edit exactly one line changed — `lexicon` — which is what identified the cause in seconds
/// once it was finally looked at.
fn moved_layers(expected: &str, actual: &str) -> Vec<String> {
    let exp: Vec<&str> = expected.lines().collect();
    let act: Vec<&str> = actual.lines().collect();
    let name_of = |l: &str| l.split(':').next().unwrap_or(l).to_string();
    let mut out = Vec::new();
    for line in &act {
        if !exp.contains(line) {
            let name = name_of(line);
            let was = exp
                .iter()
                .find(|e| name_of(e) == name)
                .map_or("(new layer)".to_string(), |e| (*e).to_string());
            out.push(format!("    {name}\n      was {was}\n      now {line}"));
        }
    }
    for line in &exp {
        let name = name_of(line);
        if !act.iter().any(|a| name_of(a) == name) {
            out.push(format!("    {name}\n      REMOVED (was {line})"));
        }
    }
    out
}

#[test]
fn bootstrap_manifest_is_pinned() {
    let actual = String::from_utf8(current_manifest()).expect("manifest is utf-8");
    if actual == EXPECTED {
        return;
    }
    let moved = moved_layers(EXPECTED, &actual);
    panic!(
        "BOOTSTRAP MANIFEST CHANGED — {} layer(s) moved:\n{}\n\n\
         Every persisted store is now unresumable (BootstrapError::ManifestDrift). If the edit is \
         intended, it is still never free:\n  \
         1. reseed — scripts/reseed-lexicon-db.sh --umls-all, then build-alignment-snapshot.sh\n  \
         2. re-point anything pinning a snapshot path (demo/prose-to-formulas-v2/run.sh)\n  \
         3. re-record any LLM draws that MISS against the changed forest, in ONE pass\n  \
         4. update EXPECTED in this file, in the SAME commit as the ontology edit",
        moved.len(),
        moved.join("\n")
    );
}

/// The diff helper itself, so a future edit to it cannot quietly stop reporting.
#[test]
fn moved_layers_reports_changed_added_and_removed() {
    let expected = "a:1\nb:2\nc:3\n";
    let actual = "a:1\nb:9\nd:4\n";
    let moved = moved_layers(expected, actual);
    let joined = moved.join("\n");
    assert_eq!(moved.len(), 3, "b changed, d added, c removed: {joined}");
    assert!(joined.contains("was b:2") && joined.contains("now b:9"));
    assert!(joined.contains("(new layer)"), "d is new: {joined}");
    assert!(joined.contains("REMOVED (was c:3)"), "c is gone: {joined}");
}
