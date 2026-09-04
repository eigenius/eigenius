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
//! IT FIRED FOR THE PROVENANCE MIGRATION (`2026-08-30`), on TEN layers, which is the whole
//! provenance axis moving off `reflection` onto `prov` plus the grade classes going away.
//! `reflection` and `prov`'s consumers (`obo`, `justification`, `statistics`, `ingest`,
//! `reference`, `lexicon`, `closed-class`, `encoding`) all reference the renamed properties;
//! `eigentt-type-fragment` moved on a single prose mention of `proof_system`, which is the kind of
//! one-line description edit that hashes exactly as loudly as a structural one. The four grade
//! classes, `epistemic_status`, the four `epistemic:*` individuals and `EpistemicStatus` are
//! deleted, `lexicon:grade` with them — 2,641,713 stamps on the converted chain, all
//! `epistemic:declared`, on resources that carry no proposition and so have no warrant to grade.
//!
//! IT FIRED ON A PROSE FIX (`2026-08-30`), on ONE layer, `lexicon`, for a description string.
//! `LexicalEntry` still advertised "and an epistemic grade" after the grade migration above deleted
//! `lexicon:grade` — the property went, the sentence promising it did not. Nothing referenced the
//! stale half, so no test caught it; it was found while checking an unrelated claim about which
//! `LexicalEntry` slots hold inductive values (two: `lexicon:cat`, `lexicon:sem_type`). Worth the
//! entry precisely because it is the cheap case: a class description is the first thing a reader
//! consults about a class, and this one named a slot that does not exist.
//!
//! IT FIRED FOR THE PROVENANCE SPLIT (`2026-08-30`), on ONE new layer, `prov`. The provenance
//! axis — Agent, Activity, the four provenance Traces and the relations between them — moves out
//! of `reflection` into its own namespace, because `reflection` had come to hold two unrelated
//! families under one word: `reflection:Trace` with LetTrace / MapTrace / CaseTrace records how a
//! PROGRAM EVALUATED, while the parentless DeclarationTrace / ObservationTrace / ProductionTrace /
//! VerificationTrace record HOW A RESOURCE CAME TO EXIST. `prov` sits ABOVE `reflection` and that
//! direction is forced: `prov:ProgramTrace` reaches into the evaluation family through
//! `prov:trace_tree` and `reflection:output`, and nothing in `reflection` reaches back. This entry
//! records only the layer's ADDITION; the migration that empties the moved declarations out of
//! `reflection` moves that layer too and is recorded separately.
//!
//! IT FIRED ON THE THREE-GROUNDS CHANGE (`2026-08-30`): `justification`, `statistics` and
//! `reflection`. `justification` is the substantive one — `justification:Term` went from seven
//! constructors to five (`DerivedEvidence` and `SpecStr` removed), `justification:Certificate`
//! lost `derived`, `sum_l`/`sum_r` now take a derivation for EACH branch, `spec_poly` dropped its
//! unchecked audit tag and leaves the term index at `j`, and `witness:IsDerivedAs` is gone. The
//! other two are description strings only, which move a hash just as surely: `statistics` and
//! `reflection` described the deleted mechanism in class descriptions, and `prov:proof_term`
//! carried the wrong account of what admits a Verified witness — a defect P3 stated in the
//! ontology and deferred to here so it could ride one reseed instead of invalidating P2's
//! mid-flight. The test named exactly the three files that were edited, which is the check that
//! the edits were the intended ones. Editing `ontologies/encoding/encoding.esl` in the same pass
//! moved NOTHING, because those edits were all `//` comments — the compiler strips them, while a
//! `description = "…"` is a resource property and hashes.
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
const EXPECTED: &str = "core:f8c7f18a36095456322d47bc5a6d26264d03e8141cf3b0320f4a7077dbcde333
eigentt-type-fragment:52bcfe935009fb7f32400dcb344ab884f29937692370aa4e3cc5a24d87250028
program:5de328f01c89486f1fac0e6be3fc44e08f0f0c886bd43305820c06a12287fde1
reflection:2455ee11766bc20134ed820e69c006951de44aa9e486abb36938d1a5361c0569
prov:742e0152373443a999e8f6562932277fe8b37da64a2ae0ce35f03ab598f9a4d9
obo:b515192765257daf466b28bb4154d6155461c8c2d1302f945ec785f8a00bb959
institution:94d7ba70bdb49cde8febceb2cef67d1421076b8c336e05cfe15f6e4c6aae263b
runtime:ada851931aeff9eed036621b306ca3eb25c0044d600c84dcad77c67973c1a22e
formulas:f7b3e06c4d26eb9fd41e3674051cc32d2277dd55a83aa6a31808e61f6d70a023
lean-expressions:edd0c69f7ac3bdd29b37318bf69a45a5860d574ad0cfc7ebe2c9c685791653b4
lean-runtime-classes:d0368fbeab60fc209aba97a41cf4ff57c25d35e954638bff26a0ffb8a0ce72cc
lean-institution:d6faf931474f38e64da8c4cafb1180eaf1dbf9800112466eb4cedc0279bbae28
justification:ee21375589e59a9cfe15e2279e8d75a9c2e706eb442d63e789525ba2f2d482b6
statistics:3ba48d9b24245a117defab3ff706945907652ce40be0a1b4a6956deb0d0070b8
notebook:0ad4665c915db5a156dbeed1fada61175fe193a0a367dbd6360fa59ebad27997
ingest:5ed296a01d68e83ba1aa2ea2a27628b5ccead88d31d060b5dd94c440246b0447
reference:33277845534074177e7c9015b0669c2fad20e35a8adc592f7df362914ecb152b
logic:eafa98fc2e8bef4d64ee96e1765a2b410219cc1025cf80e746ba4f83cf52a629
lexicon:520cf5997238198cfa1cae985f77c359c76f95f0d81562f84bd585c6c49a7061
ontology:7fb72a75946ca50e84df1aa1ae9207dc57676b96ef3c53879e82e4421f1aef43
closed-class:33288e5e89e02bdf5ae493742599a9da95d9fd949eb665831be8353a93f7eaf4
encoding:af1273fd9c4103623b79ce16a0ab33ea2269f980c7779fafc80d9a49c9b94f10
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
         4. update EXPECTED in this file, in the SAME commit as the ontology edit\n\n\
         The new manifest, ready to paste into EXPECTED:\n\
         ----8<----\n{}----8<----",
        moved.len(),
        moved.join("\n"),
        actual
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
