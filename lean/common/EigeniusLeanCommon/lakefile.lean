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
# `EigeniusLeanCommon` — hand-authored helpers the EigonFFI mirror imports

Sibling of Julia's `EigeniusJuliaCommon` (D29 §9.6). The substrate's
`LeanMirrorGenerator` emits source that calls into this package; the
spec (D30 §9.6) pins the validator semantics.

## Why hand-authored

Validators carry behavioural semantics — NaN comparison policy, regex
anchoring discipline, format dispatch — that don't fit cleanly in a
spec-driven code generator. Pinning them in a hand-authored package
keeps the generator focused on structural translation (D30 §§4–8)
and lets the validator surface evolve under conventional code-review
discipline.

## v1 surface

- `EigenValidationError` — single error type every validator throws.
- `validateMinValue` / `validateMaxValue` — IEEE-754 numeric range
  checks; NaN compares false against both bounds.
- `validateMinLength` / `validateMaxLength` — `String.length` /
  `List.length` (codepoint count, not byte count, per Lean 4 UTF-8
  semantics).
- `validatePattern` — fully-anchored regex match.
- `validateFormat` — dispatch on a `Name` symbol.
- `EigeniusUnion` — n-ary polymorphic-class field type used when a
  property's `class_types` has cardinality ≥ 2 (D30 §4.3).
-/

package EigeniusLeanCommon where

@[default_target]
lean_lib EigeniusLeanCommon where
  roots := #[`EigeniusLeanCommon]
