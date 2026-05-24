/-
Copyright 2026 The Eigenius Authors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-/

import Lake
open Lake DSL

/-!
# Phase 20a.8 capstone proof project

A small Lean project that imports a hand-authored `EigeniusFFI`
mirror stub and proves a single inequality on the mirrored
`Patient.weight` field. Build output feeds the `capstone_test`
verification flow: `lake build` → `lake exe lean4export` → bytes →
`LeanProofTerm` → kernel AutoOnLoad.

## Why a hand-rolled mirror, not the generator output

The verification side reads three things off the
`LeanPackageMirror` resource: the `library_content_hash`,
`mirrored_classes`, and `source_layer`. None of them inspect the
*shape* of the Lean source — they trust the chain's commit. So as
long as the committed mirror's archive bytes hash-match its
declared `library_content_hash` and the `mirrored_classes` list
agrees with the proposition's references, the correspondence check
passes regardless of whether the source was produced by
`LeanMirrorGenerator` or hand-authored.

For the capstone test, hand-authoring is cleaner: the test
doesn't depend on `LeanMirrorGenerator`'s output stabilising, and
the build is reproducible without re-running the Rust pipeline.
The full generator → mirror → image-build path is exercised by
`mirror_structure_lake_build` + `lean_image_build_e2e` already;
this project's job is the *verification* half of the audit chain.
-/

package CapstoneProof where

-- Path require to the workspace-vendored lean4export. The
-- capstone test runs `lake exe lean4export Capstone -- patient_weight_nonneg`
-- against this Lake project to extract the proof bytes; the
-- vendored copy is pinned (same lock the runtime worker uses) so
-- the test's output bytes are reproducible.
require lean4export from "../../runtime-worker/vendor/lean4export"

@[default_target]
lean_lib EigeniusFFI where
  roots := #[`EigeniusFFI]

@[default_target]
lean_lib Capstone where
  roots := #[`Capstone]
