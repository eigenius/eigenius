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

/-!
# `EigeniusFFI` — hand-rolled mirror stub for the Phase 20a.8 capstone

D30's `LeanMirrorGenerator` would produce this file (and three
sibling files: `lakefile.lean`, `lean-toolchain`,
`EigeniusFFI/Basic.lean`) against a synthetic chain carrying
`urn:eigenius:test:capstone:Patient` with a required Float field
under a `min_value = 0.0` constraint.

The hand-rolled version below is byte-different from what the
generator emits in its goldens, but structurally equivalent for
the capstone's purposes: a `Patient` structure with a
refinement-typed `weight : { x : Float // 0.0 ≤ x }` field, in the
`EigeniusFFI` namespace, derivable to `Repr`. The capstone test
commits a `LeanPackageMirror` resource carrying this very source as
its `library_content` archive; the verification path reads the
proposition's `EigeniusFFI.Patient` reference, finds it in the
mirror's `mirrored_classes`, and the structural correspondence
check (D28 §5.5 ¶2) passes.
-/

namespace EigeniusFFI

/-- Mirror of `urn:eigenius:test:capstone:Patient`. The `weight`
field carries a refinement (D30 §9.1) lifting the chain-side
`min_value: 0.0` constraint into Lean's type system; constructing
a `Patient` from a raw `Float` requires discharging the
nonnegativity obligation, so any `Patient` value the verifier sees
has already had that obligation discharged. -/
structure Patient where
  weight : { x : Float // 0.0 ≤ x }
  deriving Repr

end EigeniusFFI
