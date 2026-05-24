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

theorem patient_weight_nonneg : ∀ p : EigeniusFFI.Patient, 0.0 ≤ p.weight.val :=
  fun p => p.weight.property
