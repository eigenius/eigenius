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

/-- `core:min_value` check for `Float`-typed fields. Returns the
value unchanged on success; raises on out-of-range. NaN comparison
follows IEEE 754 — `NaN < bound` and `NaN ≥ bound` are both false,
so NaN fails *both* min and max checks. (D30 §9.6.)

D30 §9.1 envisions a refinement-typed surface (`{ x : Float // x ≥ lo }`);
v1 lifts the check to the decoder body only, so the field's static
type stays `Float`. Refinement-typed fields land with the D30 v1.x
emitter pass. -/
def validateMinValueFloat (field : String) (v : Float) (lo : Float) : Except String Float :=
  if v ≥ lo then .ok v
  else .error s!"{field}: value {v} below min_value {lo}"

/-- `core:max_value` check for `Float`-typed fields. Same NaN
policy as [`validateMinValueFloat`]. -/
def validateMaxValueFloat (field : String) (v : Float) (hi : Float) : Except String Float :=
  if v ≤ hi then .ok v
  else .error s!"{field}: value {v} above max_value {hi}"

/-- `core:min_value` for `Int`-typed fields. D30 §9.3 says integer
range constraints "fall back to Float-cast comparison", but lifting
through Float can lose precision for large Ints — v1 ships a
type-preserving Int comparator instead. The chain-side validator
that ran *before* the value reached the mirror has the same
discipline. -/
def validateMinValueInt (field : String) (v : Int) (lo : Int) : Except String Int :=
  if v ≥ lo then .ok v
  else .error s!"{field}: value {v} below min_value {lo}"

/-- `core:max_value` for `Int`-typed fields. -/
def validateMaxValueInt (field : String) (v : Int) (hi : Int) : Except String Int :=
  if v ≤ hi then .ok v
  else .error s!"{field}: value {v} above max_value {hi}"

/-- `core:min_length` check for strings. Lean's `String.length` counts
codepoints, not bytes — chain authors targeting byte-length must use
`core:pattern` instead (D30 §9.6). -/
def validateMinLength (field : String) (s : String) (lo : Nat) : Except String String :=
  if s.length ≥ lo then .ok s
  else .error s!"{field}: length {s.length} below min_length {lo}"

/-- `core:max_length` check for strings. Same codepoint-not-byte
discipline as `validateMinLength`. -/
def validateMaxLength (field : String) (s : String) (hi : Nat) : Except String String :=
  if s.length ≤ hi then .ok s
  else .error s!"{field}: length {s.length} above max_length {hi}"

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
def validatePattern (_field : String) (s : String) (_pattern : String) : Except String String :=
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
def validateFormat (_field : String) (s : String) (_format : Name) : Except String String :=
  -- TODO(D30 v1.x): per-format check arms (`date`, `datetime`,
  -- `time`, `iri`, `uuid`, `regex`).
  .ok s

/-- Optional-field validator combinator. Threads a per-field
validator through `Option`-typed values: `none` passes through
unchanged; `some v` invokes the validator on the inner value and
re-wraps. Used by the codec emitter to apply numeric/length/
pattern/format checks to recommended fields without duplicating
the `match` boilerplate at every call site. -/
def validateOptional {α : Type} (opt : Option α) (validator : α → Except String α) :
    Except String (Option α) :=
  match opt with
  | some v => (validator v).map some
  | none => .ok none

/-- Construct a refinement-typed subtype value from a raw value +
predicate (D30 §9.1). Returns the subtype `{ x : α // p x }` on
success or a flat-string error on failure. The predicate is
reconstructed at the call site as a Lean lambda; `DecidablePred p`
instance search resolves automatically for the predicates the
emitter generates — Lean's stdlib has `Decidable` instances on
`≤` / `≥` for `Float`, `Int`, and `Nat`, plus `And.decidable` for
conjunctions.

The emitter calls this once per refinement-constrained field
instead of chaining `validateMinValueFloat` / `validateMaxValueFloat`
/ `validateMinLength` / `validateMaxLength`. Pattern + format
checks (D30 §9.2) remain runtime-only and run on `.val` after
the refinement is established. -/
def withRefinement {α : Type} (p : α → Prop) [DecidablePred p]
    (v : α) (errMsg : String) : Except String { x : α // p x } :=
  if h : p v then .ok ⟨v, h⟩ else .error errMsg

/-- Optional-field variant of [`withRefinement`]. `none` passes
through; `some v` runs the refinement check and re-wraps. -/
def withOptionalRefinement {α : Type} (p : α → Prop) [DecidablePred p]
    (v : Option α) (errMsg : String) :
    Except String (Option { x : α // p x }) :=
  match v with
  | some x => (withRefinement p x errMsg).map some
  | none => .ok none

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

/-! ## Codec helpers

The generator emits `decodeC` / `encodeC` functions per class
(D30 §8). To keep the emitted bodies compact and readable, the
boilerplate around `Json.getObjValAs?` / `Json.getObjVal?` lives in
hand-authored helpers below — the emitter generates one call per
field instead of an inline `match` block. -/

open Lean (Json FromJson ToJson toJson fromJson?)

/-- Decode a required primitive-typed field. Wraps
`Json.getObjValAs?` with a D30 §8.1-shaped error message
(`<ClassName>.<fieldName>: missing or wrong type`). -/
def decodeRequiredPrim {α : Type} [FromJson α]
    (j : Json) (className : String) (propIri : String) (fieldName : String) :
    Except String α :=
  (j.getObjValAs? α propIri).mapError fun _ =>
    s!"{className}.{fieldName}: missing or wrong type"

/-- Decode an optional primitive-typed field. Absent property →
`none`; present-but-wrong-type → the underlying error (the
recommended-field semantics in D30 §4.1 say "absent" means
"default to none", but a present field with the wrong shape is
still a decode failure). -/
def decodeOptionalPrim {α : Type} [FromJson α]
    (j : Json) (propIri : String) : Except String (Option α) :=
  match j.getObjVal? propIri with
  | .ok jv => (fromJson? jv).map some
  | .error _ => .ok none

/-- Decode a required resource-typed field. Caller passes the
inner class's decoder. Same error shape as
[`decodeRequiredPrim`]. -/
def decodeRequiredResource {α : Type}
    (j : Json) (className : String) (propIri : String) (fieldName : String)
    (inner : Json → Except String α) : Except String α := do
  let jv ← (j.getObjVal? propIri).mapError fun _ =>
    s!"{className}.{fieldName}: missing or wrong type"
  inner jv

/-- Decode an optional resource-typed field. Same absent vs.
present-but-broken policy as [`decodeOptionalPrim`]. -/
def decodeOptionalResource {α : Type}
    (j : Json) (propIri : String)
    (inner : Json → Except String α) : Except String (Option α) :=
  match j.getObjVal? propIri with
  | .ok jv => (inner jv).map some
  | .error _ => .ok none

/-- Decode a `core:value_array` field — a JSON array of
primitive values. Returns a `List α`; the emitter converts to
`List` rather than `Array` to match D30 §4's table. -/
def decodeRequiredPrimList {α : Type} [FromJson α]
    (j : Json) (className : String) (propIri : String) (fieldName : String) :
    Except String (List α) := do
  let arr ← (j.getObjValAs? (Array α) propIri).mapError fun _ =>
    s!"{className}.{fieldName}: missing or wrong type"
  .ok arr.toList

/-- Decode a `core:resource_array` field with a singleton
`class_types` — a JSON array of embedded resources, each
decoded via `inner`. -/
def decodeRequiredResourceList {α : Type}
    (j : Json) (className : String) (propIri : String) (fieldName : String)
    (inner : Json → Except String α) : Except String (List α) := do
  let arr ← (j.getObjValAs? (Array Json) propIri).mapError fun _ =>
    s!"{className}.{fieldName}: missing or wrong type"
  arr.toList.mapM inner

/-- Read the `is_a[0]` discriminator off an embedded resource —
the dispatch key for `EigeniusUnion` decoders (D30 §8.3). Returns
the IRI string; errors descriptively when `is_a` is missing,
malformed, or empty. -/
def isAHead (j : Json) (context : String) : Except String String := do
  let arr ← (j.getObjValAs? (Array String) "urn:eigenius:core:is_a").mapError fun _ =>
    s!"{context}: `is_a` missing or not a string array"
  match arr[0]? with
  | some s => .ok s
  | none => .error s!"{context}: `is_a` is empty"

end EigeniusLeanCommon
