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

import EigeniusFFI

/-!
# Phase 20a.8 capstone proof

The single proposition the capstone end-to-end test discharges:
every `EigeniusFFI.Patient` has a non-negative weight.

The proof is trivial because the refinement-typed mirror has
*already* discharged the nonnegativity obligation at construction
time — the `weight` field is statically `{ x : Float // 0.0 ≤ x }`,
so the `0.0 ≤ p.weight.val` lemma is just the field's
`property` projection.

What the capstone tests is **not** the proof's difficulty — it's
the closed audit chain: chain class declaration → generated
mirror → in-Lean theorem → `lean4export` → `LeanProofTerm` →
kernel AutoOnLoad → nanoda verdict + three-part correspondence
check (D28 §5.5) → resource lands *verified*.

The `lean4export` output for this theorem feeds the capstone
test's `LeanProofTerm.proof_payload`; the proposition's
`EigeniusFFI.Patient` reference is what the structural
correspondence check (20a.7.x) matches against the chain-side
`urn:eigenius:test:capstone:Patient` class.
-/

theorem patient_weight_nonneg :
    ∀ p : EigeniusFFI.eigenius.test.capstone.Patient, 0.0 ≤ p.weight.val :=
  fun p => p.weight.property

/-- The notebook demo's target, and the reason it is not `patient_weight_nonneg`.

D74's statement check manufactures the Lean goal from the claim's
`reflection:canonical_proposition`, so the claim's proposition must be expressible in the §4
fragment. `∀ p, 0.0 ≤ p.weight.val` is not: `p.weight` is a structure-field access
(`PropAccess`, resource-level) and the comparison is over `Float`, which has no Lean image in v1.

This one is: `Pi` over an `EigonClass`, with an `Arrow` between two applications of an
`EigonAxiom` — every node in §4.1. It is `Prop`-valued, as Rule 21 requires of a
`canonical_proposition`, and it is provable without assuming anything, so no permitted-axiom
entry is needed. -/
theorem healthy_refl :
    ∀ (p : EigeniusFFI.eigenius.demo.lean.Patient),
      EigeniusFFI.eigenius.demo.lean.Healthy p → EigeniusFFI.eigenius.demo.lean.Healthy p :=
  fun _ h => h

/-- The notebook demo's actual target since D87 §6, and the reason `healthy_refl` is no longer it.

`healthy_refl` is a tautology over ALL Patients: its statement never mentions any particular one,
so the claim it was checked against paired a resource IRI with a proposition that said nothing
about that resource — *"any resource IRI would serve equally"*. The demo showed the plumbing ran,
not that it discriminated.

This statement is about `patient_1` and nothing else. The demo commits a second claim, identical
in shape but about `patient_2`, against this same proof; `def_eq` refuses it, which is what shows
`Holds` meaning *"this proof proves THIS claim"* rather than *"a theorem with this name
type-checks"* (eigenius#159). -/
theorem healthy_patient_1 :
    EigeniusFFI.eigenius.demo.lean.Healthy EigeniusFFI.eigenius.demo.lean.patient_1 :=
  ⟨by decide, by decide⟩

/-- The near-miss's other half, and the reason it is a theorem rather than nothing at all.

The demo checks the claim `Healthy patient_2` against the proof of `healthy_patient_1`, and that
must fail. It would fail without this declaration too — the externalizer would not find
`patient_2` among the export's names — but it would fail as `UnknownConstant`, which reads as *the
mirror is missing something*. With both individuals and both theorems exported, the near-miss
fails on `def_eq` alone: both propositions are true, both are proved, and the check still refuses
the mismatched pairing. That is the property eigenius#159 is about. -/
theorem healthy_patient_2 :
    EigeniusFFI.eigenius.demo.lean.Healthy EigeniusFFI.eigenius.demo.lean.patient_2 :=
  ⟨by decide, by decide⟩

/-- The capstone test's target, for the same reason `healthy_refl` is the demo's: the claim's
`reflection:canonical_proposition` must be expressible in D74 §4, and `∀ p, 0.0 ≤ p.weight.val`
is not. -/
theorem capstone_healthy_refl :
    ∀ (p : EigeniusFFI.eigenius.test.capstone.Patient),
      EigeniusFFI.eigenius.test.capstone.Healthy p →
        EigeniusFFI.eigenius.test.capstone.Healthy p :=
  fun _ h => h
