# Faithful Translation Specification — Eigon → Lean

**Status:** Implemented v1 (Phase 20a.0b; substrate `LeanMirrorGenerator` produces baked `EigeniusFFI` Lake packages)
**Scope:** The exact, byte-level contract between Eigon class structure (as resolved against an ontology layer in the chain) and the Lean source emitted by the substrate's mirror generator. Pins what the generator promises to produce so an auditor with the layer chain and the spec can re-derive byte-identical mirror source without reading the generator's Rust code.
**Related:**
- [D26 — Runtime Substrate](d26-runtime-substrate.md) §7 — the language-agnostic `RuntimePackageMirror` model and faithful-translation framework.
- [D28 — Lean 4 as Verification Institution](d28-lean-4-as-institution.md) §5 — the EigonFFI generator's role in the verification audit chain.
- [D29 — Eigon → Julia Faithful Translation](d29-eigon-julia-mirror-spec.md) — the sibling specification this one structurally mirrors.
- [D40 — Chain-Mirrored Lean Expressions](d40-chain-mirrored-lean-expressions.md) — the *other* Lean-side translation surface: D30 governs Lean source emission; D40 governs chain inductive values.
- [implementation-plan.md](implementation-plan.md) Phase 20a.6 — the mirror generator's implementation milestone.

The hand-authored shared package the generated source imports lives at `lean/common/EigeniusLeanCommon/` (created in Phase 20a.5).

---

## 1. Purpose

The mirror generator is **substrate Rust code** that runs inside `LeanLanguageRuntime::build_environment_image` (D26 §9.2) and implements the `MirrorGenerator` trait ([crates/runtime-substrate/src/mirror_generator.rs:78](../../crates/runtime-substrate/src/mirror_generator.rs#L78)). It walks the chain's ontology layer and emits a Lean *package* the EigonFFI consumers import — every Eigon class in the closure becomes a Lean `structure` with coercion instances, decoders, and validating constructors. The generator's output is committed back to the chain as a `LeanPackageMirror` resource (D26 §5.4 + D28 §3.1) and baked into the `LeanEnvironment` image.

Auditability of the mirror does **not** rely on the generator being a separate binary or on inspecting its Rust source. It relies on this specification: given the spec, the layer chain, and the substrate's pinned `generator_content_hash`, an auditor MUST be able to re-derive the same Lean source byte-for-byte. This document is the load-bearing TCB artifact alongside nanoda_lib and the generator binary's content hash.

When the spec and the implementation disagree, the implementation is wrong. When the spec underspecifies a behavior, both this doc and the generator are wrong — the spec is the source of truth.

The EigonFFI library is the bridge between the Eigon ontology and Lean's type theory. A Lean proof about an Eigon-side claim is **structurally a proof about the corresponding EigonFFI structure**; the three-part correspondence check (D28 §5.5) confirms that the proposition's mirror types correspond to the claim's class. If D30 is wrong, the entire verification surface is unsound — that's why it sits in the TCB alongside the term checker.

### 1.1 Conformance levels

A generator implementation conforms to **D30 v1** if it emits the constructs defined in §§4–10 for inputs in the v1 supported subset (§11) and emits a typed `MirrorGeneratorError::UnrepresentableClass` for inputs outside it. The v1 supported subset is deliberately narrower than D29 v1.2 — Lean's refinement-typed surface admits more sophisticated translations of constraint properties than Julia's runtime-validated structs, and the v1 spec covers only the slice that demonstrably round-trips through nanoda_lib's checker. The fuller surface is pinned in §11.2 as **planned** with implementation milestones.

### 1.2 Non-goals

- **Behavioural specification.** What user code *does* with EigonFFI types — proof tactics, lemma libraries, tactic-mode automation — is not the generator's concern. The generator emits only structural projection: types, coercion instances, decoders, validating constructors.
- **Round-tripping `@id`.** A resource's IRI is the chain's identity for it, not its data. Decode/encode through the mirror is intentionally lossy on `@id` (§7.4) — same discipline as D29.
- **Embedding the full Eigon validation logic in Lean's type system.** The generator emits refinement predicates where Lean-expressible (§9); constraints that don't fit Lean's refinement-type discipline (e.g. arbitrary regex `core:pattern` constraints) become runtime checks on the constructor. Static lifting of all constraints to refinement types is a research project, not v1 scope.
- **Generating proof obligations the user must discharge.** The mirror types are structural; format/constraint axioms become refinement predicates that, by construction, every decoded value satisfies (because the decoder enforced them). Users importing EigonFFI write proofs *about* mirror values that satisfy these refinements; the proof obligations the user faces are the user's own theorem statements, not generator-emitted artefacts.

### 1.3 Prior art

The "translate typed data into a proof assistant's type theory" move has a clean precedent in Lai et al.'s *Dependently Typed Knowledge Graphs* ([`lai2020dependently`]), which reproduces RDF + SPARQL in CIC/Coq with queries reformulated as types and answers as proof-carrying witnesses — the same "queries-as-types" shape this mirror serves on the Lean side. Carpenter's type-logical semantics ([`carpenter1997`]) is the compositional NL → typed-term precedent for building a term whose type is its meaning. Critically, the autoformalization-faithfulness literature (Herald [`gao2024herald`]; its audit in miniF2F-Lean Revisited [`ospanov2025minif2f`], finding ~97% LLM-judge faithfulness drops to ~66% under human evaluation; ReForm [`chen2025reform`]) shows that an LLM-judged or merely type-checking translation is *not* a faithful one — which is exactly why this spec pins a deterministic, re-derivable byte-level contract and why D28 §5.5 checks the Lean translation for *faithfulness*, not just type-correctness.

---

## 2. Output package layout

Each invocation of the generator produces exactly one Lean package, regardless of how many Eigon classes are in the closure. The package contains:

```
EigeniusFFI/
├── lakefile.lean
├── lean-toolchain
└── EigeniusFFI/
    ├── Basic.lean
    └── Mirror.lean
```

### 2.1 Package metadata (`lakefile.lean`)

The `lakefile.lean` is the literal text:

```lean
-- Auto-generated by eigon-ffi-gen — DO NOT EDIT.
import Lake
open Lake DSL

package EigeniusFFI where

require EigeniusLeanCommon from git
  "https://github.com/eigenius/EigeniusLeanCommon.git" @ "v0.1.0"

lean_lib EigeniusFFI where
  roots := #[`EigeniusFFI.Basic, `EigeniusFFI.Mirror]
```

Fields are pinned, not derived from the chain:

- `package EigeniusFFI` — package + library name are fixed for v1. Multi-mirror-per-`LeanEnvironment` support lands later (§11.2 planned).
- The `EigeniusLeanCommon` dependency is pinned to a specific version tag; the tag is part of this spec. If the hand-authored package's version changes, the spec and generator update together.
- No `lean_exe` target — the library has no binary surface.

### 2.2 Lean toolchain pin (`lean-toolchain`)

A single-line file containing the pinned Lean version, e.g.:

```
leanprover/lean4:v4.10.0
```

The version is read from the `LeanEnvironment` resource's `runtime_version` property (D28 §7.1) — pinning matches the env image's pin, so the mirror compiles against the exact toolchain the verification side will run nanoda_lib against.

### 2.3 Basic module (`EigeniusFFI/Basic.lean`)

Imports `EigeniusLeanCommon` and re-exports the helpers the generated `Mirror.lean` calls. The file is fixed v1:

```lean
-- Auto-generated by eigon-ffi-gen — DO NOT EDIT.
import EigeniusLeanCommon

namespace EigeniusFFI

export EigeniusLeanCommon (
  validateMinValue
  validateMaxValue
  validateMinLength
  validateMaxLength
  validatePattern
  validateFormat
  EigenValidationError
)

end EigeniusFFI
```

The `using` import line in `Mirror.lean` opens this namespace. New helpers in `EigeniusLeanCommon` show up here only after both the hand-authored package and this spec are updated.

### 2.4 Mirror module (`EigeniusFFI/Mirror.lean`)

The mirror module's structure, byte-by-byte:

```
-- Auto-generated by eigon-ffi-gen — DO NOT EDIT.
-- Regenerate via the substrate's image-build pipeline.
-- source_layer: <layer IRI>
-- mirrored_classes:
--   - <class IRI 1>
--   - <class IRI 2>
--   ⋮

import EigeniusFFI.Basic

namespace EigeniusFFI

<per-class blocks, in topological order — §6>

end EigeniusFFI
```

The header carries the source layer IRI and the closure's class IRIs in topological order — same content the resource's `mirrored_classes` and `source_layer` properties carry, duplicated as comments so a reader of the file alone has full provenance.

### 2.5 What's deliberately not in the output

- No `lakefile.toml` (Lake supports both formats; we pin `.lean` form).
- No tests, examples, or top-level scripts.
- No per-class file split. v1 collapses to one file because Lean parses per-module as a unit; per-class splitting is a future-work item (§11.2) when closure size makes one file unwieldy.
- No re-exports of `EigeniusLeanCommon` symbols from the `Mirror` module. User code that wants validators imports `EigeniusFFI.Basic` (which re-exports them) or imports `EigeniusLeanCommon` directly.
- No tactic-mode automation, no `@[simp]` lemmas, no instance derivation beyond what §7.5 specifies. The mirror is purely structural; proof-side ergonomics are the user's project.

---

## 3. Closure walk

Given a seed of class IRIs, the generator walks the chain to compute the *closure*: every class transitively reachable through structural references. The closure is what gets mirrored — the `mirrored_classes` property on the resulting `LeanPackageMirror` lists the closure, sorted by IRI for determinism.

The walk algorithm is **identical to D29 §3** — same closure rules, same subclass discipline, same topological order, same cycle rejection. The shared closure semantics are part of why both mirror specs are siblings: a Lean mirror and a Julia mirror for the same `seed_classes` against the same `source_layer` cover the same set of classes.

### 3.1 Reachability rules

A class C is in the closure iff:

1. C is in the seed, **or**
2. There exists a class C' in the closure such that C' has a property P (in `requires` ∪ `recommends`) with `data_type ∈ {resource, resource_array}` and `class_types` containing C.

All entries in a property's `class_types` are walked. Multi-class polymorphic properties pull every named class into the closure — none are silently dropped.

The closure is a fixpoint of these rules over the class graph, computed by a worklist algorithm seeded with the input.

### 3.2 Subclass closure

`core:subclass_of` is part of the structural closure: a class C with `subclass_of: [P]` pulls P into the closure transitively. The generator walks both edge sets in the same closure pass.

**Multi-supertype is supported.** Unlike D29 v1.2 (which rejects multi-supertype because Julia abstract types form a strict tree), Lean's `structure` mechanism supports multiple `extends` clauses cleanly. A class C with `subclass_of: [P₁, P₂]` produces a Lean structure extending both, with all parents' fields inherited and a unified field order (§5).

### 3.3 Topological order

After closure computation, classes are emitted in dependency order: a structure that has a field of type `<C>` (or `List <C>` etc.) must be defined after C. The order is determined by depth-first traversal:

1. Start from each class IRI in **sorted order** (BTreeMap key order).
2. Visit dependencies (fields whose Lean type references another mirror structure) first, recursively.
3. Append the current class to the order after dependencies are done.
4. A `visited` set short-circuits the visit so each class lands exactly once.

This produces a stable order for any closure with no cycles.

**Cyclic class references are rejected**, in two places: cycles via property `class_types` (the field-dependency graph) and cycles via `subclass_of` (the inheritance graph). Both produce `MirrorGeneratorError::UnrepresentableClass` naming the offending class. Lean's `structure` declaration requires forward references to be resolved at parse time; mutual-`structure` workarounds add nontrivial spec surface and are deferred. v1 callers must factor cycles out of the seed.

---

## 4. Faithful type translation

For each Eigon property `P` with `data_type T` (and, where relevant, `class_types` or `element_type`), the property's field in the enclosing class's mirror `structure` has the Lean type defined by this table.

| Eigon `data_type` | Side property | Lean type |
|---|---|---|
| `core:string` | — | `String` |
| `core:integer` | — | `Int` |
| `core:float` | — | `Float` |
| `core:boolean` | — | `Bool` |
| `core:json` | — | `Lean.Json` |
| `core:resource` | `class_types: [C]` (singleton) | `C` |
| `core:resource` | `class_types: [C₁, …, Cₙ]` (n ≥ 2) | `EigeniusUnion [C₁, …, Cₙ]` (see §4.3) |
| `core:resource_array` | `class_types: [C]` (singleton) | `List C` |
| `core:resource_array` | `class_types: [C₁, …, Cₙ]` (n ≥ 2) | `List (EigeniusUnion [C₁, …, Cₙ])` |
| `core:value_array` | `element_type: core:string` | `List String` |
| `core:value_array` | `element_type: core:integer` | `List Int` |
| `core:value_array` | `element_type: core:float` | `List Float` |
| `core:value_array` | `element_type: core:boolean` | `List Bool` |
| `core:value_array` | `element_type: core:json` | `List Lean.Json` |

Order of `class_types` IRIs in an `EigeniusUnion` is the IRI sort order (canonical for determinism), same convention as D29 §4.

### 4.1 Required vs. recommended

- A property in `core:requires` produces a **required field** with the bare type from the table.
- A property in `core:recommends` produces an **optional field** with the type `Option T`, defaulting to `none`.

A property listed in both `requires` and `recommends` is malformed at the ontology level; the generator's behavior is unspecified.

### 4.2 Subclass coercions

For each class C with `subclass_of: [P₁, …, Pₘ]`, the generator emits one `instance` declaration per parent making C a `Coe` target of P:

```lean
instance : CoeOut C P₁ where
  coe c := { /* fill P₁'s fields from c.toP₁ */ }
```

Lean's `extends` mechanism produces these implicitly when the subclass extends the parent (`structure C extends P₁ where ...`); the generator emits explicit `CoeOut` instances on top for the direction Lean's elaborator doesn't auto-derive (the implicit subclass-coercion is `Coe`, which is contravariant for function arguments — `CoeOut` covers the dual case). Cross-class compatibility with the chain's "instance of subclass is instance of parent" semantics requires both directions.

### 4.3 Polymorphic `class_types` — `EigeniusUnion`

Multi-class `class_types` (n ≥ 2) renders as `EigeniusUnion`, a Lean-side helper defined in `EigeniusLeanCommon`:

```lean
inductive EigeniusUnion : List Type → Type
  | inl : (h : T) → EigeniusUnion (T :: ts)
  | inr : (rest : EigeniusUnion ts) → EigeniusUnion (T :: ts)
```

(Effectively a chain of `Sum` types, indexed by a class list for ordered decoding.) Generated structures with `Union`-typed fields embed via the n-ary constructors `EigeniusUnion.inl₀ x`, `EigeniusUnion.inl₁ x`, …; the decoder dispatches on the chain-side `is_a` of the embedded `ResourceRef` value.

### 4.4 Unsupported `data_type` values

Any `data_type` not in the §4 table — including future kernel types and `core:resource` / `core:resource_array` with empty `class_types` — produces a `MirrorGeneratorError::UnrepresentableClass` naming the offending class and property.

---

## 5. Structure field ordering

A class's structure has its required fields first (in the order declared in `requires`), followed by its recommended fields (in the order declared in `recommends`). Inherited fields from parents land **before** own fields, in parent-declaration order.

Within `requires` / `recommends`, the order is **the order the property IRIs appear in the chain's resource representation** — same convention as D29 §5. The kernel's canonical Resource representation uses BTreeMap-sorted property keys; practical ontology authors get IRI-sorted ordering, and ontologies that explicitly serialise an ordered `requires` array get that order preserved.

---

## 6. Per-class emission

For each class C in topological order, the module body contains four definitions, separated by single blank lines:

1. The `structure C` declaration (§7).
2. Coercion instances for each `subclass_of` parent (§4.2).
3. `decodeC : Lean.Json → Except String C` (§8.1).
4. `encodeC : C → Lean.Json` (§8.2).

Each block ends with a single trailing newline, then a blank line before the next. There is no `Mirror`-level `export` declaration — Lean modules export everything in their namespace by default; consumers `import EigeniusFFI.Mirror` and reach symbols via the full path.

---

## 7. Structure declaration

### 7.1 Structure shape

For a class C with required properties `q₁, …, qₙ` and recommended properties `r₁, …, rₘ`, with parents `P₁, …, Pₖ`:

```lean
/-- <core:description, if present> -/
structure C extends P₁, P₂, …, Pₖ where
  q₁ : T₁
  q₂ : T₂
  ⋮
  qₙ : Tₙ
  rⱼ : Option Tⱼ := none  -- for each recommended
  deriving Repr, Inhabited
```

`Repr` is derived so debug printing works without ceremony. `Inhabited` is derived when every required field has a `Default` instance reachable from `EigeniusLeanCommon` or `core` — when it can't be derived (e.g. a class with a required field of class type `C'` that itself has no `Inhabited` instance), the generator emits the structure without the `Inhabited` deriving and accepts the consequence (no default value). Falling back to omitting `deriving` rather than synthesising a fake default is the safer move — code that depends on `Inhabited` will fail to compile clearly rather than silently round-trip through a meaningless zero value.

### 7.2 Reserved `_id` field

Every generated structure includes a hidden `_id : Option String := none` field positioned first (before all required fields). This carries the resource's `@id` through decode/encode round-trips (§8.4). Properties named `_id` in the chain ontology are rejected at resolution time (§11.1) — same reservation as D29.

### 7.3 Empty class

A class with no `requires`, no `recommends`, and no parents produces:

```lean
structure C where
  _id : Option String := none
  deriving Repr, Inhabited
```

The structure is non-trivial because of the reserved `_id` field. The `Inhabited` instance is derivable trivially.

### 7.4 `@id` semantics

`@id` is information about a resource's identity, not its data. The mirror's decode reads `@id` if present, stashes it in `_id`. Encode writes it back. Two structurally-identical mirror values with different `_id`s are *not* equal under Lean's structural equality — a `_id`-aware lemma must compare on `c.toData` (where `toData` strips `_id`) if proofs want to reason modulo identity.

In practice, proofs about mirror values written via EigonFFI quantify over the data fields and ignore `_id`. The `_id` round-trip exists so encode-decode is the identity on `Lean.Json` shape (modulo `@id` rewriting through the validator) — important for the closed-audit-chain property where the substrate-emitted `LeanProofTerm` resource carries the exact bytes that reproduce verification.

### 7.5 Derived instances

Beyond `Repr` (always) and `Inhabited` (when derivable), the generator emits one `instance` per chain-declared `subclass_of` parent giving the explicit `CoeOut C P` direction (§4.2). No other instances are derived in v1. `DecidableEq` is *not* derived — equality on mirror structures depends on equality of every field (including `Lean.Json`-typed fields where decidability is non-trivial), and forcing the derivation gives the user a compile error in surprising places. Users who need `DecidableEq` for specific mirror types declare it themselves.

---

## 8. JSON codec emission

The chain's wire form for embedded resources is Eigon-JSON. The mirror's codec converts between `Lean.Json` and the structure type. v1 uses `Lean.Json` rather than CBOR because Lean has first-class JSON support in `core` (`Lean.Json` + `Lean.fromJson?`); CBOR support requires an external dependency that adds dual-version complexity to the toolchain pin. The substrate's CBOR wire is converted to/from JSON at the institution boundary.

### 8.1 `decodeC : Lean.Json → Except String C`

For each class C, the decoder reads the IRI-keyed property map. Required fields fail with a structured `Except` error if missing; recommended fields default to `none` when absent. Class-typed fields recurse into the referenced class's decoder.

Shape (for a class C with required `weight : Float` whose Eigon IRI is `urn:project:weight`):

```lean
def decodeC (j : Lean.Json) : Except String C := do
  let _id ← match j.getObjValAs? String "@id" with
    | .ok v => pure (some v)
    | .error _ => pure none
  let weight ← j.getObjValAs? Float "urn:project:weight"
    |>.mapError fun _ => "C.weight: missing or wrong type"
  return { _id, weight }
```

The error message format is fixed: `<ClassShortName>.<fieldShortName>: <reason>`. Reasons are pinned per failure mode (`missing or wrong type`, `validation failed: <constraint>`, `unknown discriminator`).

### 8.2 `encodeC : C → Lean.Json`

For each class C, the encoder writes the IRI-keyed property map. The `_id` field, if `some s`, lands as `"@id" := s`. The class's `is_a` lands as a single-element JSON array containing the class's IRI (the mirror is single-class-typed at encode time; chain-side multi-is_a is handled by the substrate's epistemic-stamping pass, not by the mirror).

Shape:

```lean
def encodeC (c : C) : Lean.Json := Json.mkObj <|
  (match c._id with | some v => [("@id", Json.str v)] | none => [])
  ++ [
    ("urn:eigenius:core:is_a", Json.arr #[Json.str "<C's full IRI>"]),
    ("urn:project:weight", Json.num (JsonNumber.fromFloat c.weight)),
    -- ...
  ]
```

Fields appear in **encode order**: `@id` first (if present), then `is_a`, then required fields in declaration order, then recommended fields (only those with `some v`) in declaration order. This is the canonical Eigon-JSON shape the kernel emits for a resource.

### 8.3 Polymorphic field codecs

For a field typed `EigeniusUnion [C₁, …, Cₙ]`:

- **Decode**: read the embedded resource's `is_a[0]`, dispatch to `decodeCᵢ` for the matching class. If the discriminator is none of the n classes, return `Except.error "field <name>: unknown discriminator: <iri>"`.
- **Encode**: dispatch on the `EigeniusUnion` ctor — `inl x` writes `encodeCᵢ x`.

### 8.4 Information preservation across decode/encode

`decodeC ∘ encodeC = pure` on all v1-supported inputs, modulo:

- `recommends` fields explicitly set to `none` decode-then-encode as absent (the `@id` field follows the same discipline).
- Property ordering follows §8.2's canonical encode order, regardless of input order on decode.
- `@id` round-trips intact when present.

The generator emits a per-class round-trip test in `EigeniusLeanCommon/Test/` (a separate package compiled but not shipped) for every class in the closure — this is part of the integrity chain (§10.2).

### 8.5 Module-level codec registry

The mirror module exports two constants the institution's commit-time translator uses to dispatch decoders/encoders by class IRI:

```lean
def eigeniusDecoders : Std.HashMap String (Lean.Json → Except String _) :=
  Std.HashMap.ofList [
    ("urn:project:Patient", fun j => Sigma.mk _ <$> decodePatient j),
    -- ...
  ]
```

The `Sigma` envelope erases the per-class type so the registry has a uniform value type. The substrate-side dispatcher uses the existential to dispatch on string-keyed IRIs.

---

## 9. Validating constructors

Constraint properties on the property definition produce **refinement-typed structures** for the subset of constraints expressible in Lean's type system, and **runtime checks** for the rest. The split is pinned in this section.

### 9.1 Constraints lifted to refinement predicates

| Constraint property | Lean refinement |
|---|---|
| `core:min_value` (numeric) | `field : { x : T // x ≥ <lit> }` |
| `core:max_value` (numeric) | `field : { x : T // x ≤ <lit> }` |
| `core:min_length` (string) | `field : { s : String // s.length ≥ <lit> }` |
| `core:max_length` (string) | `field : { s : String // s.length ≤ <lit> }` |

These constraints land as Lean *subtype* declarations on the field. The decoder validates and packages the value into the subtype; downstream proofs can quote the refinement as a hypothesis.

When **multiple** value-range constraints apply to the same field (e.g. both `min_value` and `max_value`), the refinement is the conjunction:

```lean
field : { x : Float // 0.0 ≤ x ∧ x ≤ 100.0 }
```

The refinement order is fixed: min-value, max-value, min-length, max-length (numeric first, length second). Multi-constraint refinements emit `∧` chains in this order.

### 9.2 Constraints kept as runtime validators

| Constraint property | Validator call |
|---|---|
| `core:pattern` | `validatePattern <fieldStr> "<escaped pattern>" >>= fun _ => …` in the decoder |
| `core:format` | `validateFormat <fieldStr> <formatSymbol> >>= fun _ => …` |

Patterns and format predicates are kept runtime because:

- Lifting regex matching to a refinement type would require a decidable-equality predicate over `String × Regex` — possible (Lean has `decideable_regex` libraries) but adds a TCB component the v1 spec deliberately avoids.
- `core:format` IRIs like `:date` or `:iri` admit known-shape regex implementations on both sides (Lean and the kernel-side Rust validator) but the spec for what "valid date" means is the validator code, not a Lean theorem.

Constraint properties not in §9.1 or §9.2 (extensions of the kernel's constraint vocabulary) are passed through to `validateFormat` with the constraint property's IRI as the symbol value — same fall-through pattern as D29 §9.3. The validator MUST raise on unknown constraints.

### 9.3 Numeric literal rendering

`min_value` / `max_value` are rendered as Lean `Float` literals in all cases — `0` becomes `0.0`, `100.0` stays `100.0`, `0.5` stays `0.5`. The validator's parameter type is `Float`; integer fields fall back to `Float`-cast comparison at decode time (an integer-typed range constraint is upcast to Float for the refinement, then narrowed back to Int after the check passes).

`min_length` / `max_length` are rendered as Lean `Nat` literals.

### 9.4 Pattern handling

The pattern string is rendered as a Lean-escaped double-quoted string literal (backslashes doubled, double quotes escaped, newlines as `\n`, `\$` escaped because Lean's string macros recognise `$` for antiquotation). The pattern is passed *verbatim* to `validatePattern`; anchoring is the validator's job (same anchored-match discipline as D29 §9.4 — `validatePattern` constructs the regex as fully-anchored).

### 9.5 Format symbol rendering

If the property's `core:format` is `urn:eigenius:core:formats:<name>`, the format argument is the Lean syntax `` `<name> `` (a `Name` literal). v1 recognised values: `` `date ``, `` `datetime ``, `` `time ``, `` `iri ``, `` `uuid ``, `` `regex `` — matching the core ontology's `core:formats:*` enumeration.

A `core:format` IRI **outside** the `urn:eigenius:core:formats:` prefix is **passed through to the validator as a `Name` constructed from the full IRI** (`Name.mkSimple "<full IRI>"`). The validator MUST raise on unknown formats; same passthrough policy as D29 §9.3.

### 9.6 Validator semantics (delegated to `EigeniusLeanCommon`)

The validators called by generated source live in `EigeniusLeanCommon/EigeniusLeanCommon.lean`. Their semantics, pinned here so the spec is closed:

- `validateMinValue` / `validateMaxValue`: compare-by-value; raises `EigenValidationError` on out-of-range. NaN comparison follows IEEE 754 (NaN compares false against both bounds → both raise).
- `validateMinLength` / `validateMaxLength`: dispatched via `String.length` / `List.length`. Note that Lean's `String.length` is **codepoint count, not byte count** (Lean 4 strings are UTF-8 with codepoint indexing) — chain authors targeting byte-length checks must use `core:pattern` instead. Same discipline as D29.
- `validatePattern`: **fully-anchored** match. The validator constructs the regex as `^(?:pattern)$` framing and uses Lean's regex library (`Regex.matches`). Same discipline as kernel-side and D29-side.
- `validateFormat`: dispatch on the format symbol. Each known format has a purpose-built check. Unknown formats raise `EigenValidationError`.

### 9.7 Regex syntax — portability boundary

Pattern strings appear at validation time in three places: the kernel-side validator (Rust `regex` crate), the Julia-side validator (Julia `Regex`), and the Lean-side validator (Lean's regex library). The three engines accept overlapping but non-identical syntax. The v1 portable subset is the same ECMA-262 features pinned in D29 §9.5. Lean's regex library is PCRE-derived — same constraints as Julia.

---

## 10. Determinism and integrity chain

### 10.1 Determinism

Same `(generator binary, source layer, seed class IRIs)` produces byte-identical output. Sources of determinism:

- BTreeMap is used for every property iteration in the kernel's resource representation.
- Closure walk uses a sorted worklist seeded from sorted seed IRIs.
- Topological sort visits classes in BTreeMap key order, breaking ties by IRI.
- `class_types` IRIs are sorted before `EigeniusUnion` ordering (§4).
- Field-order within `requires` / `recommends` follows chain order (§5).
- File contents in the `LibraryContent::Embedded` archive are emitted in declaration order (`lakefile.lean`, `lean-toolchain`, `EigeniusFFI/Basic.lean`, `EigeniusFFI/Mirror.lean`); the substrate's hash function (§10.2) sorts by path for the digest.
- No timestamp, hostname, machine state, or wall-clock value appears in the output.

### 10.2 Integrity chain

The `LeanPackageMirror` resource the generator commits carries three pinned identifiers (same shape as D29 §10.2):

- `generator_identifier` = `"eigon-ffi-gen"`.
- `generator_version` = the `eigenius-lean-runtime` crate's `Cargo.toml` version. Synthesised at compile time via `env!("CARGO_PKG_VERSION")`.
- `generator_content_hash` = `sha256:<hex>` where the hex is the SHA-256 digest of `"eigon-ffi-gen:<version>"`. v1 placeholder; a future spec version replaces it with the SHA-256 of the generator's compiled binary or its `Cargo.lock` digest.

The `library_content_hash` is the SHA-256 over a length-prefixed framing of the library archive's bytes:

```
For each (path, content) pair in path-sorted order:
  big-endian u64 (path.len()) || path || big-endian u64 (content.len()) || content
```

The hash digests the full archive — `lakefile.lean`, `lean-toolchain`, `EigeniusFFI/Basic.lean`, `EigeniusFFI/Mirror.lean` all contribute.

### 10.3 Mirror IRI

The committed `LeanPackageMirror` resource's `@id` is derived from the `library_content_hash`:

```
urn:eigenius:runtime:mirror:lean:<first 16 hex chars of the digest>
```

Same 16-hex-char prefix convention as D29 §10.3. Two byte-identical mirrors produce identical IRIs — chain dedupe is intentional.

### 10.4 Generator integrity tests

The generator's release pipeline runs three integrity tests against the canonical core ontology:

1. **Round-trip golden file.** Regenerate the Core mirror, diff against the checked-in golden, assert byte-equality.
2. **Lake compile.** The generated source compiles cleanly via `lake build` against the pinned Lean toolchain. No warnings (any deprecation surfaced as a warning is a generator bug — bump the spec and the generator together).
3. **Decode-encode round-trip.** For each mirrored class, a hand-crafted Eigon-JSON resource decodes via `decodeC`, encodes via `encodeC`, the result is structurally equal to the input modulo §8.4's field-ordering canonicalisation.

These are part of the v1 conformance bar; a generator that fails any of them is non-conformant.

---

## 11. v1 supported subset and planned extensions

### 11.1 v1 supported subset

A class declaration is in the v1 supported subset iff **all** of the following hold:

- All its `requires` and `recommends` properties have `data_type` in the §4 table (excluding the empty-`class_types` case for `resource` / `resource_array`).
- `class_types` lists are non-empty when used.
- `element_type` for `value_array` is in `{string, integer, float, boolean, json}`.
- The closure does not contain a cycle in property `class_types` references (§3.3).
- The class's `subclass_of` chain is acyclic. Multi-supertype is supported (§3.2), unlike D29 v1.2.
- The transitive field set (own + inherited via `subclass_of`) has unique property `short_name`s. Two distinct property IRIs with the same `short_name` produce a duplicate Lean structure field — rejected. The reserved `_id` slot counts in this check; properties declared with `short_name = "_id"` are rejected.
- All class and property `short_name` values are valid Lean identifiers (alphanumeric + underscore, leading non-digit; Lean's identifier syntax is the same restriction as Julia's). Capital-letter discipline is enforced for class names (Lean structure names must start with a capital letter), lowercase for property names (Lean field names must start with a lowercase letter).
- Format IRIs on `core:format` are either standard (`urn:eigenius:core:formats:<name>`, rendered as `` `<name> ``) or any other IRI the validator can dispatch on (rendered as `Name.mkSimple "<full IRI>"`, §9.5).
- The mirror module exports `eigeniusDecoders` (class IRI → Sigma-erased decoder) per §8.5.

A class outside the v1 supported subset produces `MirrorGeneratorError::UnrepresentableClass` with a message naming the offending class and the rule it violated.

### 11.2 Planned extensions

These are part of the long-term D28 §5 contract but require generator features that have not yet been implemented:

| Item | Implementation milestone | Spec version |
|---|---|---|
| `core:allows_only` enum support (Lean `inductive` with one ctor per allowed value) | Phase 20b | D30 v1.1 |
| Embedded resources (`Value::Embedded`) decode/encode | Phase 20b | D30 v1.1 |
| `core:pattern` as a refinement type via decidable-regex library | Future-work | D30 v2 |
| `core:format` as decidable predicates (date/time/UUID structurally) | Future-work | D30 v2 |
| Multi-mirror per `LeanEnvironment` (per-env package name) | Future-work | D30 v2 |
| Per-class file split | Future-work | TBD |
| Generator binary content hash (real SHA-256, not version-string placeholder, §10.2) | Future-work | TBD |
| Mutually-recursive structures (cyclic class graphs) via Lean's `mutual` blocks | Future-work | D30 v2 |
| `Inhabited` synthesis for classes without natural defaults (via per-mirror axioms) | Future-work | TBD |

A generator implementation MAY implement any combination of planned items ahead of the milestone, provided it does so according to the relevant spec version's text.

---

## 12. Spec versioning

This document is a draft v1 of D30. Every change to the contract is a spec version bump:

- **Patch versions (1.0.x)** — clarifications that don't change generator output or the integrity chain.
- **Minor versions (1.x.0)** — additive features (new `data_type` mappings, new validators, new closure rules) where pre-bump output remains a valid subset of post-bump expected output.
- **Major versions (2.0.0)** — breaking changes that re-shape existing output. Existing mirrors become invalid; chain audits use the spec version pinned by the `generator_version` on the mirror resource.

D30 v1 is the spec version for `eigenius-lean-runtime 0.1.x` from Phase 20a.6 onward.

---

## 13. Open questions

These are decisions the spec deliberately defers; resolving them produces a v1.x or v2 bump.

- **`Float` semantics in Lean.** Lean's `Float` is IEEE-754 double, same as Julia. v1 inherits IEEE 754's discipline including NaN propagation. A separate "no-NaN" predicate is out of scope.
- **`Lean.Json` for value-typed fields.** v1 uses `Lean.Json` for `core:json` fields. A more typed alternative (a structured `EigeniusJson` inductive) would let proofs quantify over JSON shape; deferred to v2 when a consumer needs it.
- **Coercion direction asymmetry.** §4.2 emits `CoeOut C P`; Lean's `extends` mechanism gives `Coe C P` automatically via field projection. Whether both directions need explicit instances or whether `extends` alone is sufficient depends on how downstream proofs invoke coercion — pinned to "both for safety" in v1, may relax in v2 once usage patterns settle.
- **`DecidableEq` derivation policy.** v1 doesn't derive `DecidableEq` (§7.5). Some users will want it. A future spec version may opt-in via a per-class hint property on the chain (`lean:derive_decidable_eq: true`).
- **Refinement-type discharge in user proofs.** When a user proves a theorem about an `EigeniusFFI.Patient` value, the refinement-typed fields carry their constraints in the type. Whether the spec mandates a particular shape for the refinement-discharge helpers (e.g. `Patient.weight_in_range`) is open — leaving it to user-side ergonomics for now.

These items are surfaced for review but do not block v1 conformance — generator implementations and the chain agree by following v1's pinned defaults.
