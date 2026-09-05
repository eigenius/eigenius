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
capstone test's above. Both live here because both consumers share this one Lake project.

**Two `Nat` fields, and none of the three choices is incidental.**

*Fields at all*, unlike the capstone's refinement-typed `weight`: the demo needs NAMED INDIVIDUALS
so a claim's proposition can be *about* its subject (D87 §6), and constructing one means
discharging every field's obligation. `0.0 ≤ (x : Float)` cannot be discharged in the kernel —
`Float.ble` is `@[extern]`, so `Float.le` does not reduce and `rfl` proves nothing about it — so a
refinement here would leave the individuals unconstructible. The refinement's own story, D30 §9.1
lifting a chain-side `min_value` into Lean's type system, is told by
`eigenius.test.capstone.Patient` above, whose `patient_weight_nonneg` proves exactly that.

*`Nat` and not `Float`*: `Nat` comparison reduces in the kernel, so a proposition about these
fields is provable by `decide`. Every `Float` operation is `@[extern]` and reduces to nothing.

*More than zero fields*: a nullary structure makes `patient_1` and `patient_2` DEFINITIONALLY
EQUAL — structure eta gives `{} = {}` — so no claim about one could be told from a claim about the
other, which is the property the demo exists to show. -/
structure eigenius.demo.lean.Patient where
  /-- Chart identifier. Distinct values are what make two individuals distinguishable. -/
  chartId : Nat
  /-- Resting heart rate, beats per minute. -/
  restingHr : Nat
  deriving Repr

/-- Mirror of the chain axiom `urn:eigenius:demo:lean:Healthy : demo:Patient -> Prop`, and it says
something ABOUT its argument.

It was `fun _ => True` — constant, ignoring the patient — which is a deeper version of the defect
D87 §6 names. With a constant predicate `Healthy patient_1` and `Healthy patient_2` are both
definitionally `True`, so `def_eq` accepts either claim against either proof and NO arrangement of
subjects can make the demo discriminate. Measured before the change: the near-miss verdict came
back `Holds`.

A `def` and not a Lean `axiom`, as before: an axiom would have to be named in the institution's
permitted-axiom allowlist, and a proof that assumes its own predicate demonstrates nothing. -/
def eigenius.demo.lean.Healthy (p : eigenius.demo.lean.Patient) : Prop :=
  50 ≤ p.restingHr ∧ p.restingHr ≤ 100

/-- A named individual. The demo's claim is *about* this one, which is what `patient_1` failed to
be while it was a chain resource with a ∀-quantified proposition hanging off it: that proposition
never mentioned it, so any resource IRI would have served equally (D87 §6). -/
def eigenius.demo.lean.patient_1 : eigenius.demo.lean.Patient :=
  { chartId := 1, restingHr := 62 }

/-- The second individual, and the near-miss's subject. Equally real and equally healthy: the demo
proves `Healthy patient_1` and `Healthy patient_2` both, then binds the proof of the first to the
claim about the second. Both propositions being true is what leaves the statement comparison as
the only thing that can refuse it. -/
def eigenius.demo.lean.patient_2 : eigenius.demo.lean.Patient :=
  { chartId := 2, restingHr := 71 }

/-- The capstone test's counterpart of the above, over its own namespace's `Patient`. Both exist
because both consumers need a claim whose proposition is inside D74's §4 fragment, and each
names its own chain class. -/
def eigenius.test.capstone.Healthy (_p : eigenius.test.capstone.Patient) : Prop := True

end EigeniusFFI
