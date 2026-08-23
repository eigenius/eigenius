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
//! IT FIRED FOR eigenius#188 A SECOND TIME (`2026-08-23`), on **`core` and
//! `eigentt-type-fragment`**. The level algebra moved DOWN to `core:Level` and `core:result_sort`
//! was retyped from a string (`"Prop"` / `"Set"` / `"Type:N"`) to a `core:Level` value. It had to
//! move: `core:Asserts` carries a `result_sort`, so the property is used inside `core` itself, and
//! a lower layer cannot reference a higher one — the same constraint that stopped `eigentt:Level`
//! reusing `lean:LeanLevel`. One algebra now serves both `result_sort` and `eigentt:TypeExpr`'s
//! `Sort` ctor, and `data X : Sort u` is expressible where the string grammar could not spell a
//! level variable. `core` moving revalidates every layer above it; both reseeds fold into one,
//! since none has run since the first move.
//!
//! IT FIRED FOR eigenius#188 (`2026-08-23`), on ONE layer, `eigentt-type-fragment`: the
//! `TypeExpr` `Sort` constructor's argument changed from `core:integer` to a new
//! `eigentt:Level` inductive (Zero/Succ/Max/IMax/Param), so a universe level can be a `Max`,
//! an `IMax` or a `Param` instead of only a numeral. The decoder still accepts the old numeral
//! form — `decodes_the_pre_188_numeral_form` pins that — so a persisted store's terms remain
//! readable; what it cannot survive is this manifest move, which is why the reseed is owed
//! regardless. **eigenius#213 rides along with that reseed** rather than paying its own.
//!
//! IT FIRED ON THE D73 CLOSE-OUT (`2026-08-22`): two layers, `program` (eigenius#210 declared
//! `program:components:RunRuntimeScript`, which D56 §7 added to the kernel's REMOTE_COMPONENTS
//! and never declared here, so the kernel could dispatch a component no chain could reference)
//! and `reflection` (eigenius#205 added `ExternalExecutionTrace`, the Declared-admitting sibling
//! of `ProgramTrace`). Folded into ONE reseed with #210's vocabulary work rather than paying two.
//!
//! IT FIRED AGAIN (`2026-08-21`), on the D73 batch: three layers at once — `reflection`
//! (eigenius#200 relaxed `VerificationTrace.derivation_trace` to `recommends` and widened two
//! descriptions), `reasoning` (eigenius#203 retired the `spec_str` rule) and `encoding`
//! (eigenius#201 made `enc:EncodedClaim` a `DeclaredResource`). Batching the three into one reseed
//! is the whole reason #196 groups them; the test named exactly the three that were edited and
//! nothing else, which is the check that the edits were the intended ones.
//!
//! IT HAD FIRED IN ANGER BEFORE (`2026-08-20`). D71 slice 7 added the `formalize` cell type to the
//! notebook ontology — a change whose whole point is that it is deliberate, since
//! `notebook:cell_type` carries `allows_only` precisely so a new cell type cannot appear by
//! accident. The test named the one moved layer and its four-step follow-through, and the reseed
//! was paid knowingly rather than discovered days later by a broken demo.
//!
//! WHAT IT DOES NOT COVER. A snapshot also goes stale when the LEXICON content changes with no
//! bootstrap edit — the 2026-08-15 atom overrides dropped `drug target` and added `synthetic lethal`,
//! no manifest moved, and recorded draws still missed. Catching that needs a recorded fingerprint of
//! the lexicon an experiment's draws were made against; the reseed's `PROVENANCE` stamp is half of it.

use eigenius_kernel::bootstrap::current_manifest;

/// The manifest as committed. Update it in the SAME commit as any bootstrap ontology edit — see the
/// panic message for the rest of the follow-through.
const EXPECTED: &str = "\
core:2069508694585321fc035cb6b442f27459a06ddd50b1fd9e77d542cb1444609d
eigentt-type-fragment:68d5552fb2901a845e5a09fa19d4ceb11d7152e19d251562b5bdc76591e124f6
program:224bb234a8651afdeb5144dca0e609afded5a633dd6495f4ae588e44bf855d4e
reflection:c4c613c9b8391371f6c3346c2248f79f846ae674f5022b629665eee556cdb9a8
obo:b0fccf59c68bc65d7b311d4a02d500b6ce2aba908a1824856392188130de1ddf
institution:27871f87612484d6469b66a4b0379731152af0c1f8eb04007c8b0343f4648c13
runtime:4c05dc3b114acb2554e8f8d594a6878f94e8f60f3a512d46eb816d3a030f26cc
formulas:ba63b387d496f46effd86b6e544c2daebea69b605c5b366123f17317bbae7957
lean-expressions:6263f64c4fb167dedb9ba69c2e353517bd343b21da7ad6aa346f42f2b975fac5
lean-runtime-classes:11de512ae4aea72e0865a19becdefd4daed9c9f6cdf2abd5af3b88d17078294e
lean-institution:e48be69b9df06f02232feac048fc4ec5bdebcf62b8a50b90145b9f62176610dc
reasoning:d1564233f6e7e889a107b30b0cb709e8576baf42f12f700f407a70648413b548
statistics:0e179b5ef88c9e01399a84d6c863b6ddb4fe38859374c45b3f685175d584af3b
notebook:376c2726292782680b5534319890b63af0a70d6e8f5938972be6ee2d30d635e9
ingest:67534b5bcf3478a18bd5df2a3c856132e8702dd3f9d3e8727b40179794ae0aba
reference:3df0920240eac8e23d79ab4892b58052f2a273897cd73c8a39ae1c75b647e372
logic:bde155e5644cb03e236cd94a501301b878832707d0ba9c6a361fa204ce9e813d
lexicon:51aa4972dd065159a0124ed22e776819a9016d92a4fdfb4d8841ad56581e81fa
ontology:12321a1bb48ad9f89071cd6500a0791f4a33a60ced9c4fdaf0e82e6dae9faa70
closed-class:7c15f2350e7de9e4e1af0291f2cedc2753867265841d09c3be7192dc7f91813d
encoding:eba8626de510a15d3c66811db50847b7a6bd93ed4d39eb9718d04368a6ba608a
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
