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

import Lean

/-- Manual `Repr` for `Lean.Json`. D30 §4 maps `core:json` to
`Lean.Json` and §7.5 mandates `deriving Repr` on every mirror
structure. Lean.Json doesn't ship with a derived `Repr` instance
and `deriving instance` rejects it on parser grounds — we instead
write the instance by hand using `Json.compress`, which renders
the value as a single-line JSON string. Compact rather than
pretty-printed because `Repr` consumers usually want a one-liner
they can paste into a `#eval` for round-trip debugging. -/
instance : Repr Lean.Json where
  reprPrec j _ := Std.Format.text j.compress

/-!
# `EigeniusLeanCommon` — hand-authored helpers the generated EigonFFI
mirror calls.

D30 §9.6 pins the contract: every validator throws
`EigenValidationError` on failure, and the spec text is the source of
truth for each validator's semantics. A future spec version may
expand the surface; for v1 the eight symbols below are the entire
externally-visible API.

We import `Lean` (above) so the generated `Mirror.lean` — which
imports `EigeniusFFI.Basic` which in turn imports this module — sees
`Lean.Json` in scope without an extra import directive. D30 §4's
`core:json` mapping references the same path; keeping the import here
keeps the codec-emitter output minimal.
-/

namespace EigeniusLeanCommon

/-- Error every validator raises on failure. Carries the field name
the value belonged to and a human-readable reason. Surface is
deliberately flat (no error codes) — the decoder wraps these into
`Except String` for the chain-side dispatcher. -/
structure EigenValidationError where
  field : String
  reason : String
  deriving Repr

instance : ToString EigenValidationError where
  toString e := s!"{e.field}: {e.reason}"

/-- Common `Except` shape the validators return. Wrapping `Either`
matches what `decodeC` in the generated mirror chains over with
`>>= fun _ => …`. -/
abbrev EigenValidation (α : Type) := Except EigenValidationError α

/-- `core:min_value` refinement check. Returns the value unchanged on
success; raises on out-of-range. NaN comparison follows IEEE 754 —
`NaN < bound` and `NaN ≥ bound` are both false, so NaN fails
*both* min and max checks. (D30 §9.6.) -/
def validateMinValue (field : String) (v : Float) (lo : Float) : EigenValidation Float :=
  if v ≥ lo then .ok v
  else .error { field, reason := s!"value {v} below min_value {lo}" }

/-- `core:max_value` refinement check. Same NaN policy as
`validateMinValue`. -/
def validateMaxValue (field : String) (v : Float) (hi : Float) : EigenValidation Float :=
  if v ≤ hi then .ok v
  else .error { field, reason := s!"value {v} above max_value {hi}" }

/-- `core:min_length` check for strings. Lean's `String.length` counts
codepoints, not bytes — chain authors targeting byte-length must use
`core:pattern` instead (D30 §9.6). -/
def validateMinLength (field : String) (s : String) (lo : Nat) : EigenValidation String :=
  if s.length ≥ lo then .ok s
  else .error { field, reason := s!"length {s.length} below min_length {lo}" }

/-- `core:max_length` check for strings. Same codepoint-not-byte
discipline as `validateMinLength`. -/
def validateMaxLength (field : String) (s : String) (hi : Nat) : EigenValidation String :=
  if s.length ≤ hi then .ok s
  else .error { field, reason := s!"length {s.length} above max_length {hi}" }

/-- `core:pattern` check — fully-anchored regex match.

v1 stub: D30 §9.6 mandates anchored matching, but Lean's stdlib has
no regex engine and pulling one in expands the verification-side
TCB. The structural pipeline lands first; lighting up real pattern
matching is a follow-up that uses Lean's `Regex` library (Mathlib's
`Mathlib.Data.Regex` or `leanprover-community/regex`) once the
toolchain pin permits.

Until then this is a permissive validator — accepts everything,
returns the string. The runtime check is preserved on the
*kernel* side via the Rust `regex` crate before any value reaches a
Lean mirror, so a permissive Lean-side check doesn't reduce the
verification surface, it only loses one layer of defence-in-depth.

A failing-closed alternative would reject everything (spec-correct,
useless in practice); a feature-flag would let downstream
deployments opt in once they pull in a regex dep. v2 settles this. -/
def validatePattern (_field : String) (s : String) (_pattern : String) : EigenValidation String :=
  -- TODO(D30 v1.x): wire to a real anchored-match implementation
  -- once the spec settles on a regex library dependency.
  .ok s

/-- `core:format` dispatch. Each known format has a purpose-built
check; unknown formats raise (D30 §9.6).

v1 stub mirrors `validatePattern`: the structural pipeline lands
without enforcing format-specific predicates, so the per-format
shape (date / datetime / iri / uuid / regex) doesn't need to be
authored before the generator can emit calls into it. Adding a
specific format check is a single-arm extension when the verifier
of a downstream proof depends on it.

The kernel-side validator already runs every chain-side
constraint before a value lands in a mirror, so a permissive
Lean-side check is defence-in-depth only, not the soundness floor. -/
def validateFormat (_field : String) (s : String) (_format : Name) : EigenValidation String :=
  -- TODO(D30 v1.x): per-format check arms (`date`, `datetime`,
  -- `time`, `iri`, `uuid`, `regex`).
  .ok s

/-- N-ary polymorphic-class field type — the Lean translation of an
Eigon property with `class_types` cardinality ≥ 2 (D30 §4.3).

Carried indexed by the class list so the decoder can dispatch on the
embedded resource's `is_a[0]` to the correct constructor:

```lean
inductive EigeniusUnion : List Type → Type
  | inl : (h : T) → EigeniusUnion (T :: ts)
  | inr : (rest : EigeniusUnion ts) → EigeniusUnion (T :: ts)
```

The generated `Mirror.lean` emits `EigeniusUnion.inl x` / iterated
`.inr` for each class position; downstream proofs pattern-match on
the chain of `inl`/`inr` constructors. -/
inductive EigeniusUnion : List Type → Type 1
  | inl : {T : Type} → {ts : List Type} → T → EigeniusUnion (T :: ts)
  | inr : {T : Type} → {ts : List Type} → EigeniusUnion ts → EigeniusUnion (T :: ts)

/-- Position-only `Repr` for `EigeniusUnion`. Renders the chain of
`inl`/`inr` constructors so debug output shows which arm of the
union a value sits in; the inner payload is **not** rendered.

Rationale: `EigeniusUnion` lives at universe `Type 1` (it carries an
arbitrary `T : Type`), and Lean can't derive `Repr` for
`Type 1`-indexed inductives automatically. A full position-+-payload
`Repr` would need a `Repr T` instance for every type in the list,
which the generator can't promise (the user may decline to derive
`Repr` on a specific class, and decidability of inner-type Repr is
not a closure-walker concern).

The position-only output keeps `deriving Repr` working on every
mirror structure that has a union field — without it, the
generator would have to skip `Repr` on those structures, breaking
D30 §7.5's "always derive `Repr`" promise. Users who need a richer
Repr for a specific union write their own instance. -/
private def reprPos : {ts : List Type} → EigeniusUnion ts → String
  | _, .inl _ => "inl"
  | _, .inr rest => "inr." ++ reprPos rest

instance : Repr (EigeniusUnion ts) where
  reprPrec u _ := Std.Format.text s!"EigeniusUnion.{reprPos u}"

end EigeniusLeanCommon
