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
core:b49fc5afbe26277ffc0da764b5f7022dbd51fc291229cd4c138f9993b4cae87b
eigentt-type-fragment:304d1a49596612b0a202b5c6417e1e57fd5fa8545a1baf83e7ef8348f627082f
program:5de328f01c89486f1fac0e6be3fc44e08f0f0c886bd43305820c06a12287fde1
reflection:1396f4a1438a465a654d9db38018fac9276d876a6b5d9303c42d435b10799cb3
obo:cb157be55245c9ac73385e0ac0c605f0cde8ecb9d1246b07226008cbf8cf4d76
institution:94d7ba70bdb49cde8febceb2cef67d1421076b8c336e05cfe15f6e4c6aae263b
runtime:5ef02306d25bc1b52e517114c8b7f2280ca20ad1d70607255198b4d54d8b4bfc
formulas:2073d36b31311e89803a78f20dc28f376197d89eb75a384dbd429d2203455817
lean-expressions:2b084735d270d8d1078e69e67de342b5d221f5f7a4939598582190f4c017d8f6
lean-runtime-classes:d0368fbeab60fc209aba97a41cf4ff57c25d35e954638bff26a0ffb8a0ce72cc
lean-institution:41c7dce62971e48bd5bdd08e97bac59e72f27fc38a7e36c4afdfcb3edc0e8d34
reasoning:fb5c7ec72dcd308581729c25916f2ef4813ebd0581b370c9cf735107c385f144
statistics:7f250b815205a6e08e55ee240c96bea112ee51dad03f16caa98be63339f7d2aa
notebook:0ad4665c915db5a156dbeed1fada61175fe193a0a367dbd6360fa59ebad27997
ingest:6b3b1de79a7b69ffa60337824a82598b26fa2e99166a18f19bb27f1d07e72b0a
reference:f57afb5d36519ef867ebc8ac974b744992eb0fcf70b45f0cda68c6265976c1fd
logic:e23ffb70b63f80cea1a7287fa67f47d34f13270118e9c72f5904814470cec36a
lexicon:f83f332a9a2ae455d9341de17a5930b22324c462ab7e514e29395ec3d3d8432a
ontology:7fb72a75946ca50e84df1aa1ae9207dc57676b96ef3c53879e82e4421f1aef43
closed-class:cbac8f32e15ae7b013a723ccca044dc165099fd8edc7204a7255f095bb04dbc2
encoding:b59e72bb3da7147f1f1dcb28d8aaece30f6a1349476aadf001fc04e834cf6d90
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
