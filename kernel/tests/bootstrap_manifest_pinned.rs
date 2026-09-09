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
//! families under one word: `program:traces:Trace` with LetTrace / MapTrace / CaseTrace records how a
//! PROGRAM EVALUATED, while the parentless DeclarationTrace / ObservationTrace / ProductionTrace /
//! VerificationTrace record HOW A RESOURCE CAME TO EXIST. `prov` sits ABOVE `reflection` and that
//! direction is forced: `prov:ProgramTrace` reaches into the evaluation family through
//! `prov:trace_tree` and `program:traces:output`, and nothing in `reflection` reaches back. This entry
//! records only the layer's ADDITION; the migration that empties the moved declarations out of
//! `reflection` moves that layer too and is recorded separately.
//!
//! IT FIRED ON THE THREE-GROUNDS CHANGE (`2026-08-30`): `justification`, `statistics` and
//! `reflection`. `justification` is the substantive one — `justification:Term` went from seven
//! constructors to five (`DerivedEvidence` and `SpecStr` removed), `justification:Grounds`
//! lost `derived`, `sum_l`/`sum_r` now take a derivation for EACH branch, `instantiate` dropped its
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
//! IT FIRED ON THE INDEX-LANGUAGE RESIDUE (`2026-09-09`), on ONE layer, `core`. The rename below
//! left three sentences standing that still asserted an index: two `witness:Is*As` descriptions
//! ending "recomputed every time and persisted nowhere, so the index is a cache" and one reading
//! "the index over it is a cache". Each sentence's first half is true and its second half names a
//! structure that does not exist, which is why a residue grep on "witness index" did not see them.
//!
//! IT FIRED ON THE `implicit(T, P)` CHANGE (`2026-09-09`), on ONE layer, `justification`.
//! `justification:Grounds.instantiate` declares `implicit(T, P)`, so the constructor's binder list
//! changed and the declaration hashes differently. The ten call sites that lost two arguments each
//! are chain content and move nothing here. The D89 entry in the work stack had listed this as
//! deliberately deferred, because it needed the unifier's scope check to stop identifying generated
//! variables by name; that fix landed in the same commit.
//!
//! IT FIRED ON THE WITNESS-ADMISSION RENAME (`2026-09-09`), on TWO layers, `core` and
//! `justification`. Description text only, and false text rather than merely imprecise: three
//! `witness:Is*As` descriptions in `core` said the kernel synthesizes a witness "when the layer's
//! witness index holds a matching key", and `justification:Grounds` said its grounding constructors
//! consume a witness the kernel synthesizes "from the layer's witness index". D66 slice 0 removed
//! that map; the lookup is a decision procedure over Trace resources. A class description is what a
//! reader consults first, so one naming a structure that does not exist is the same defect as the
//! `lexicon:grade` entry above.
//!
//! NONE OF THE THREE ABOVE WAS COVERED BY THE D89 RESEED (`2026-09-08`), which ran before all of
//! them. That reseed's two snapshots and the docker volume it filled are stale on `core` and
//! `justification` until the next one runs.
//!
//! IT FIRED ON THE `justification.esl` REWRITE (`2026-09-07`), on ONE layer, `justification`.
//! The file's comments and `description` properties were rewritten to state the current design
//! rather than the sequence of edits that produced it, and to frame it on
//! `docs/design/judgements-and-warrants.tex` — the paper this branch implements — rather than on
//! the D-documents that preceded it. No declaration changed: same seven constructors, same
//! signatures, same `requires`/`recommends`, same properties. The hash moved because a
//! `description` is a resource property and hashes, while a `//` comment is stripped. Nothing
//! else in the manifest moved, which is the check that the edit was confined to prose. One
//! substantive correction rode along: a comment asserted that `justification:Conclusion` "stays
//! `subclass_of reflection:DerivedResource`" — the class declares no parent, and that class was
//! deleted.
//!
//! IT FIRED ON B6 (`2026-09-06`), on ONE layer, `core`. Two constructor arguments on
//! `eigentt:Term` were retyped from `core:string` to `core:iri`: `ConstRef.iri` and
//! `CtorApp.decl_iri`. Both name a declaration and always did; B3 declared three OTHER leaves
//! IRI-valued (`Certificate.declared`/`.observed`/`.verified`, the `witness:Is*As` index,
//! `Term.Checked.payload_iri`) and did not reach these, which went unnoticed because the mentions
//! walker recovered them by matching `urn:` and so never needed the declaration. B6 removes that
//! heuristic, and the retype is what keeps a `ConstRef` target a dependency once it is gone.
//! `CtorApp.ctor_name` deliberately stays `core:string`: constructors have no chain-resolvable
//! identity (D79 §2.2.1), so it names no declaration. The reseed this obliges is B4, which the
//! entry below already owes.
//!
//! IT FIRED AGAIN ON B1 (`2026-09-05`): `core` and `justification`. `core` gained the
//! `core:implicit_args` property and a `recommends` on `core:InductiveCtor`; `justification`
//! declares `app`'s `A`/`B` and `sum_l`/`sum_r`'s `P` implicit, which changes those
//! constructors' chain-resident declarations. Both are content, so both hash. The reseed this
//! obliges is B4 in `docs/notes/next-steps-after-d88.md`, which also carries B2's merge and the
//! three stale description strings from `#235`.
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
const EXPECTED: &str = "core:38aa65a9c6cbacc8a8434c0e30d8048e28a7d1f5d09743fa4006af44f381bf7e
program:429718a323b6bfcc3ff858277f73b2c15de724f9d1c1c2c2c220748295b3c726
program-traces:89a26cb0570d90ac8e0943687cc0f175e1cd1ea78a025196a247b636d1440f7a
prov:694b3195028f88f8043209f81f70824fb12bdee45f8e57db381a041c96687c5d
obo:b515192765257daf466b28bb4154d6155461c8c2d1302f945ec785f8a00bb959
institution:149ab16a9b3d48e3839a1881d1a72230d7c88d1281f109c58eb8b5f944e68379
runtime:ada851931aeff9eed036621b306ca3eb25c0044d600c84dcad77c67973c1a22e
formulas:f7b3e06c4d26eb9fd41e3674051cc32d2277dd55a83aa6a31808e61f6d70a023
lean-runtime-classes:d0368fbeab60fc209aba97a41cf4ff57c25d35e954638bff26a0ffb8a0ce72cc
lean-institution:3a4cd1b1a75a5032fda484dea529bcf79678ec4d4fd060e13c6ef00d782e5fc2
justification:3544a09a5fa235332df6bfb40bc0c68ce266b72704d86d85a08d6a63f785ae2f
statistics:8c955279b691648a948061c47cd8ff2ada4c9edd9a05c756b373a5aa83c3b466
notebook:0ad4665c915db5a156dbeed1fada61175fe193a0a367dbd6360fa59ebad27997
ingest:5ed296a01d68e83ba1aa2ea2a27628b5ccead88d31d060b5dd94c440246b0447
reference:dfc95385753cf9d829bb527271bd12ad898f76075b86cd10c4ff3575baaf1852
logic:eafa98fc2e8bef4d64ee96e1765a2b410219cc1025cf80e746ba4f83cf52a629
lexicon:7ee38132e0b9d11e8ef91d88a8bcc8f81996715f66effd46e22c41ee6df80d7f
ontology:7fb72a75946ca50e84df1aa1ae9207dc57676b96ef3c53879e82e4421f1aef43
closed-class:a691050fcb75087947ef1b6c426b35b9ca872b9b1c8b7b76d020d4518463acee
encoding:a7ce37f8cbf5b7ef3d34895c63098c1f5d1076adaaec67f250317a987e5c8d5a
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
