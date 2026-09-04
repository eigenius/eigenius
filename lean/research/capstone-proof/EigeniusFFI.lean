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
has already had that obligation discharged.

Named as D74 §3.3's mangling spells the IRI, since eigenius#208: a class's Lean name is its
namespace path plus its `core:short_name`. The flat `Patient` this replaced is what collided. -/
structure eigenius.test.capstone.Patient where
  weight : { x : Float // 0.0 ≤ x }
  deriving Repr

/-- Mirror of `urn:eigenius:demo:lean:Patient` — the notebook demo's own class, distinct from the
capstone test's above. Both live here because both consumers share this one Lake project. -/
structure eigenius.demo.lean.Patient where
  weight : { x : Float // 0.0 ≤ x }
  deriving Repr

/-- Mirror of the chain axiom `urn:eigenius:demo:lean:Healthy : demo:Patient -> Prop`.

A `def`, not a Lean `axiom`, deliberately: an axiom would have to be named in the institution's
permitted-axiom allowlist, and more to the point a proof that ASSUMES its own predicate
demonstrates nothing. The body is irrelevant to the statement being checked — `def_eq` compares
`Healthy p -> Healthy p` on both sides and never needs to unfold it. -/
def eigenius.demo.lean.Healthy (_p : eigenius.demo.lean.Patient) : Prop := True

end EigeniusFFI
