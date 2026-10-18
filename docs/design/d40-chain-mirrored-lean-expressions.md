# D40: Chain-Mirrored Lean Expressions

**Date:** 2026-05-19
**Status:** Implemented v1 (Phase 20a.0; `lean:LeanExpr` / `lean:LeanLevel` / `lean:LeanName` bootstrap inductives + Lean→chain mirror in production)
**Prerequisites:** D14 (Institution Realisation), D19 (EigenTT Inductive Types), D26 (Runtime Substrate), D28 (Lean 4 as Verification Institution), D32 (Chain-Mirrored EigenTT Inductives + FormulaTerm)
**Drives:** Phase 20a (the first complete Lean institution — `lean:LeanExpr` is the queryable shape of `LeanProofTerm.proposition`).

## 1. Motivation

### 1.1 The queryable-proposition problem

Phase 20a's `LeanProofTerm` resource carries a Lean proof end-to-end: from Lean source, through `lean4export`, onto the chain, and through nanoda_lib for re-checking. The verification verdict comes from nanoda parsing the verbatim export bytes — that's the load-bearing path and it stays bytes-shaped forever (D28 §6.3).

But the Eigenius value proposition is "knowledge graph + typed reasoning over committed resources." A verification artifact stored as opaque bytes can be re-checked, but it can't be *queried* — an EigenQL pattern like "find every Theorem in the chain whose proposition asserts a bound on function `f`" needs the proposition to be a structured chain resource. Without that, the only way to inspect proposition structure is to parse the bytes via a Lean checker, which moves audit reasoning out of the graph and into external code.

The solution: **chain-mirror Lean expressions as a typed InductiveType**. The proposition (the *type* of the target Theorem, one Expr tree) lives on the chain as a `lean:LeanExpr` value alongside the verbatim proof bytes. EigenQL queries traverse the inductive value directly; cross-institution Comorphisms can refer to proposition structure structurally; audit tooling never leaves the graph.

This is the D32 move applied to Lean. D32 brought `FormulaTerm` (a EigenTT fragment) onto the chain so cross-institution comorphisms could transfer formulas structurally. D40 brings Lean's expression form onto the chain so propositions become first-class queryable resources.

### 1.2 What gets mirrored — and what doesn't

The mirror is **propositions only** (the target Theorem's *type*), not whole proof terms. Three reasons (carried forward from D28 §6.3):

1. **Proofs are huge.** A Mathlib-scale theorem's proof term, with its transitive declaration closure, runs to thousands of expressions sharing common subterms via the export-format's back-reference DAG. Eigon resources are trees (cross-resource references exist, intra-resource sharing doesn't). Chain-mirroring proofs would require either (i) thousands of top-level resources per proof or (ii) a sharing primitive Eigon doesn't have. Verbatim bytes get sharing for free via the export format's back-references.

2. **Translators are TCB.** The chain-mirror translator is in the verification institution's TCB — a bug there is a soundness bug. The translator's domain matters: translating *one* Expr (a proposition) is small enough to specify exhaustively and audit; translating thousands (a whole proof) widens the TCB surface for no query gain.

3. **Re-checking is bytes-only.** nanoda parses the verbatim export bytes. The chain-mirrored proposition is for *queries and audits*, never for re-checking. Treating proofs the same way as propositions would tempt future code to re-check from the mirror — a soundness hazard the design explicitly forecloses.

So D40 specifies four chain inductives: `lean:LeanExpr` (the proposition shape), `lean:LeanLevel` (universe levels appearing in `Sort`), `lean:LeanLevelList` (the cons-list carrier for `Const`'s universe-instantiation array — §3.3), and `lean:LeanName` (Lean's dotted-name hierarchy used by `Const` references). The `LeanProofTerm.proposition` field carries a `lean:LeanExpr` value; nanoda's `Expr` decodes into it; queries walk it.

### 1.3 Why D40 is not D32

D32 brought a EigenTT *fragment* onto the chain — the symbol-algebra-relevant subset (`Var`, `LitFloat`, `OpRef`, `App`, `Lam`, `Pi`). D40 brings Lean's *full term language* onto the chain (modulo `Local`, see §3.3). The difference matters:

- D32's FormulaTerm is engineered for cross-institution agreement — Symbolics, JuMP, IntervalArithmetic, DiffEq, Catalyst all consume the same shape. The ctor list is the *minimum* common subset.
- D40's `LeanExpr` is engineered for *Lean's* expressivity. It must round-trip every Lean Expr that can appear in a closed proposition — universes, structure projections, primitive literals, dependent binders with `BinderStyle`.

D32 is a *language*. D40 is a *language mirror*. The chain hosts both, the chain inductive mechanism handles both, but their design pressures are different.

## 2. Background

### 2.1 nanoda_lib's `Expr` is the data we mirror

nanoda's `Expr` ([references/nanoda_lib/src/expr.rs:21](../../references/nanoda_lib/src/expr.rs#L21)) is the data structure the Lean term checker manipulates. Eleven variants:

```rust
pub enum Expr<'a> {
    Var { dbj_idx: u16, .. },
    Sort { level: LevelPtr<'a>, .. },
    Const { name: NamePtr<'a>, levels: LevelsPtr<'a>, .. },
    App { fun: ExprPtr<'a>, arg: ExprPtr<'a>, .. },
    Pi { binder_name, binder_style, binder_type, body, .. },
    Lambda { binder_name, binder_style, binder_type, body, .. },
    Let { binder_name, binder_type, val, body, nondep, .. },
    Proj { ty_name: NamePtr, idx: usize, structure: ExprPtr, .. },
    Local { binder_name, binder_style, binder_type, id: FVarId, .. },
    StringLit { ptr: StringPtr<'a>, .. },
    NatLit { ptr: BigUintPtr<'a>, .. },
}
```

Plus auxiliary types: `Level` ([level.rs](../../references/nanoda_lib/src/level.rs)) with five ctors (`Zero`, `Succ`, `Max`, `IMax`, `Param`), and `Name` ([name.rs](../../references/nanoda_lib/src/name.rs)) with three ctors (`Anon`, `Str`, `Num`). Each `Expr` carries cached hash bits and `num_loose_bvars` / `has_fvars` flags (computed, not stored).

The export format's JSON shape (lean4export semver 3.1.x) is a DAG-encoded version of this structure: a name table, a level table, an expression table, each entry referring to earlier table entries by index. Back-references compress the shared subterms.

### 2.2 nanoda doesn't preserve sharing on parse

When nanoda parses the export format into in-memory `Expr` trees, it materialises the back-references into a `LeanDag` ([util.rs](../../references/nanoda_lib/src/util.rs)) — a hash-cons'd table where structurally identical Exprs share storage. The DAG is internal to nanoda; what the user code sees is `ExprPtr`s that look like pointers into the DAG.

For our purposes (chain-mirroring propositions), the proposition is the *type* of a single target Theorem — one root Expr. Sharing within that one Expr tree is bounded (a proposition is much smaller than the proof bytes), so mirroring the tree as a chain inductive value without explicit sharing is feasible. We make this concrete in §4.2.

### 2.3 The chain mirror surface (D32 mechanics)

D32 established the pattern: chain InductiveTypes are declared as `core:InductiveType` resources with `core:InductiveCtor` ctors, each carrying `core:InductiveArgType` argument shapes. Values are committed as Eigon-CBOR with the tagged-dict shape D19/D32 specify. The validator type-checks inductive values at commit. The mirror generator (D29 for Julia, D30 for Lean) emits language-side bindings.

D40 uses this surface directly. The three new InductiveTypes (`lean:LeanExpr`, `lean:LeanLevel`, `lean:LeanName`) commit to a `urn:eigenius:lean:` ontology layer alongside the institution's other declarations (D28 §10.3). They participate in the standard validator + mirror generation pipeline — no new chain mechanics.

## 3. Design — the four inductives

We build up bottom-up: `lean:LeanName` is the simplest, depends on nothing; `lean:LeanLevel` depends on `lean:LeanName` (`Param` carries a name); `lean:LeanExpr` depends on both.

### 3.1 `lean:LeanName`

Lean's name space is hierarchical: a name is a sequence of either string or numeric components, written `Foo.bar.42`. nanoda's `Name` ([name.rs](../../references/nanoda_lib/src/name.rs)) mirrors this with three ctors:

```rust
pub enum Name {
    Anon,                     // the empty name
    Str(NamePtr, StringPtr),  // prefix . string-suffix
    Num(NamePtr, u32),        // prefix . numeric-suffix
}
```

Chain mirror:

```json
{
  "@id": "urn:eigenius:lean:LeanName",
  "core:is_a": ["core:InductiveType"],
  "core:short_name": "LeanName",
  "core:description":
    "Mirror of Lean's hierarchical name. Mirrors nanoda's `Name` one-for-one. Names are formed from `Anon` (the empty name) plus a sequence of string or numeric suffix applications. `Foo.bar` is `Str(Str(Anon, \"Foo\"), \"bar\")`; `Foo.42` is `Num(Str(Anon, \"Foo\"), 42)`.",
  "core:ctors": [
    "urn:eigenius:lean:ctor:Name.Anon",
    "urn:eigenius:lean:ctor:Name.Str",
    "urn:eigenius:lean:ctor:Name.Num"
  ]
}
```

Ctor shapes:

| Ctor | Args | Mirrors |
|---|---|---|
| `Name.Anon` | (none) | `Name::Anon` |
| `Name.Str` | `prefix: LeanName`, `suffix: string` | `Name::Str(NamePtr, StringPtr)` |
| `Name.Num` | `prefix: LeanName`, `suffix: integer` | `Name::Num(NamePtr, u32)` |

Each ctor's `InductiveArgType` entries carry `core:arg_name` (per D32 §3.2's optional name property) so the chain shape is self-describing. `core:type_name` on the recursive slot is `urn:eigenius:lean:LeanName` (the self-reference is permitted, §3.5 below).

### 3.2 `lean:LeanLevel`

Lean's universe levels appear in `Sort u` expressions and in `Const`'s universe-instantiation list. nanoda's `Level` ([level.rs](../../references/nanoda_lib/src/level.rs)) has five ctors:

```rust
pub enum Level {
    Zero,
    Succ(LevelPtr),
    Max(LevelPtr, LevelPtr),
    IMax(LevelPtr, LevelPtr),
    Param(NamePtr),
}
```

`Zero` is the bottom (`Prop` lives at `Sort Zero` for definite-prop levels, `Sort (Succ Zero)` for `Type 0`, etc.). `Succ`, `Max`, `IMax` compose. `Param` is a universe-level variable (Lean's universe polymorphism mechanism).

Chain mirror:

```json
{
  "@id": "urn:eigenius:lean:LeanLevel",
  "core:is_a": ["core:InductiveType"],
  "core:short_name": "LeanLevel",
  "core:description":
    "Mirror of Lean's universe level. Mirrors nanoda's `Level` one-for-one. `Zero` is the prop universe; `Succ u` lifts; `Max`/`IMax` compose; `Param` introduces a universe-level variable for universe-polymorphic constants.",
  "core:ctors": [
    "urn:eigenius:lean:ctor:Level.Zero",
    "urn:eigenius:lean:ctor:Level.Succ",
    "urn:eigenius:lean:ctor:Level.Max",
    "urn:eigenius:lean:ctor:Level.IMax",
    "urn:eigenius:lean:ctor:Level.Param"
  ]
}
```

Ctor shapes:

| Ctor | Args | Mirrors |
|---|---|---|
| `Level.Zero` | (none) | `Level::Zero` |
| `Level.Succ` | `base: LeanLevel` | `Level::Succ(LevelPtr)` |
| `Level.Max` | `left: LeanLevel`, `right: LeanLevel` | `Level::Max(LevelPtr, LevelPtr)` |
| `Level.IMax` | `left: LeanLevel`, `right: LeanLevel` | `Level::IMax(LevelPtr, LevelPtr)` |
| `Level.Param` | `name: LeanName` | `Level::Param(NamePtr)` |

### 3.3 `lean:LeanLevelList` — cons-list carrier for `Const.levels`

A standalone two-ctor inductive that carries the universe-instantiation list on `LeanExpr.Const`. The early draft of D40 proposed `Const.levels: core:value_array<lean:LeanLevel>`, but Phase 20a.2's implementation work surfaced that the chain's `core:element_type` property is constrained to primitive types (`allows_only: [string, integer, float, boolean, json]`, per [`ontologies/core/core-ontology.json`](../../ontologies/core/core-ontology.json) §element_type) — a value-array of inductive elements wouldn't validate. The clean alternative is a chain-side cons-list inductive, declared inside `lean-expressions` itself.

```json
{
  "@id": "urn:eigenius:lean:LeanLevelList",
  "core:is_a": ["core:InductiveType"],
  "core:short_name": "LeanLevelList",
  "core:description":
    "Cons-list carrier for the universe-instantiation array on `LeanExpr.Const`. Two ctors — `Nil` and `Cons(head: LeanLevel, tail: LeanLevelList)`.",
  "core:ctors": [
    "urn:eigenius:lean:ctor:LevelList.Nil",
    "urn:eigenius:lean:ctor:LevelList.Cons"
  ]
}
```

Ctor shapes:

| Ctor | Args | Mirrors |
|---|---|---|
| `LevelList.Nil` | (none) | empty `LevelsPtr` |
| `LevelList.Cons` | `head: LeanLevel`, `tail: LeanLevelList` | `LevelsPtr` element prepend |

The translator (§4.1) converts between this cons-list and nanoda's flat `Arc<[LevelPtr]>` shape via `iter().rev().fold(Nil, |acc, l| Cons(l, acc))` and the reverse. The encoding overhead — N+1 nested `Cons` resources per N-element universe list — is bounded by Lean's typical usage: most consts carry 0 universe parameters (`Nil`), some carry 1 (`Cons(u, Nil)`), few carry 2+. Even Mathlib-scale propositions rarely exceed depth-3 universe lists.

### 3.4 `lean:LeanExpr`

The main inductive. **Ten ctors**, mirroring nanoda's `Expr` minus `Local`.

Why omit `Local`? `Local` represents a free variable introduced during nanoda's traversal (when checking a binder, the body is "instantiated" by replacing the de Bruijn index with a Local marker carrying the binder's type). Committed proofs are **closed terms** — no Locals appear in them. The export format never serialises a Local. nanoda fabricates Locals during checking but they don't cross the institution boundary; including a `LeanExpr.Local` ctor would be ceremony that never fires. If a future need arises (proof-state inspection beyond closed propositions), the ctor can be added in a v2 spec.

```json
{
  "@id": "urn:eigenius:lean:LeanExpr",
  "core:is_a": ["core:InductiveType"],
  "core:short_name": "LeanExpr",
  "core:description":
    "Mirror of Lean's expression form, minus `Local` (closed terms only — `Local` is fabricated by nanoda during traversal and never crosses the institution boundary). Mirrors nanoda's `Expr` ctors one-for-one otherwise. Ten ctors: `Var` (de Bruijn bound variable), `Sort` (universe expression), `Const` (named environment reference with universe instantiation), `App`, `Pi`, `Lambda`, `Let`, `Proj` (structure projection), `StringLit`, `NatLit`.",
  "core:ctors": [
    "urn:eigenius:lean:ctor:Expr.Var",
    "urn:eigenius:lean:ctor:Expr.Sort",
    "urn:eigenius:lean:ctor:Expr.Const",
    "urn:eigenius:lean:ctor:Expr.App",
    "urn:eigenius:lean:ctor:Expr.Pi",
    "urn:eigenius:lean:ctor:Expr.Lambda",
    "urn:eigenius:lean:ctor:Expr.Let",
    "urn:eigenius:lean:ctor:Expr.Proj",
    "urn:eigenius:lean:ctor:Expr.StringLit",
    "urn:eigenius:lean:ctor:Expr.NatLit"
  ]
}
```

Ctor shapes:

| Ctor | Args | Mirrors |
|---|---|---|
| `Expr.Var` | `dbj_idx: integer` | `Expr::Var { dbj_idx }` |
| `Expr.Sort` | `level: LeanLevel` | `Expr::Sort { level }` |
| `Expr.Const` | `name: LeanName`, `levels: LeanLevelList` | `Expr::Const { name, levels }` |
| `Expr.App` | `fun: LeanExpr`, `arg: LeanExpr` | `Expr::App { fun, arg }` |
| `Expr.Pi` | `binder_name: LeanName`, `binder_style: string`, `binder_type: LeanExpr`, `body: LeanExpr` | `Expr::Pi { ... }` |
| `Expr.Lambda` | `binder_name: LeanName`, `binder_style: string`, `binder_type: LeanExpr`, `body: LeanExpr` | `Expr::Lambda { ... }` |
| `Expr.Let` | `binder_name: LeanName`, `binder_type: LeanExpr`, `val: LeanExpr`, `body: LeanExpr`, `nondep: boolean` | `Expr::Let { ... }` |
| `Expr.Proj` | `ty_name: LeanName`, `idx: integer`, `structure: LeanExpr` | `Expr::Proj { ty_name, idx, structure }` |
| `Expr.StringLit` | `value: string` | `Expr::StringLit { ptr }` |
| `Expr.NatLit` | `value: string` | `Expr::NatLit { ptr }` |

Notes on field-shape choices:

- **`dbj_idx` is `core:integer`, not `core:string`.** nanoda uses `u16` internally; the chain's `core:integer` covers `i64`, so `u16` round-trips with room. The chain validator doesn't enforce the `u16` upper bound (65535) — overflow would require a proof more deeply nested than any realistic Lean program; not a v1 concern.
- **`binder_style` is `core:string`**, one of `default` / `implicit` / `strictImplicit` / `instImplicit`. The enum is small and chain-side enumerable; v1 carries it as a tagged string for simplicity. A future spec version may promote it to a named inductive (`lean:LeanBinderStyle`) when a cross-institution use case calls for typed discrimination on binder style.
- **`NatLit.value` is `core:string`** (decimal digits), not `core:integer`. Lean's `NatLit` carries arbitrary-precision naturals via nanoda's `BigUintPtr` (`num_bigint::BigUint`); the chain's `core:integer` is `i64` and overflow's a real concern (`Nat` values in mathematical proofs routinely exceed `i64::MAX`). The string-of-digits encoding is unambiguous and round-trips losslessly. Validators enforce `^[0-9]+$` syntax.
- **`StringLit.value` is `core:string`** — passed through verbatim. The chain's CBOR encoding is UTF-8 so Lean's UTF-8 string literals round-trip identity. (Lean does allow embedded null bytes in `String`s; nanoda preserves them via `StringPtr`. The chain's CBOR carries arbitrary UTF-8 byte sequences including nulls.)
- **`Const.levels` is `lean:LeanLevelList`** (a chain-side cons-list inductive, §3.3) rather than `core:value_array<LeanLevel>`. A Const reference with no universe parameters encodes as `Nil`. The order matters (Lean's universe-instantiation is positional) and the cons-list preserves it.

### 3.5 Why `BinderStyle` is a string, not an inductive

Lean's binder styles are presentation-only (pretty-printer + elaborator preference); they don't affect type-checking. nanoda parses and preserves them, but they're never load-bearing for soundness. Encoding the style as a `core:string` keeps the proposition shape small (one field rather than a `Sum`-like inductive nested in every binder) and matches how the export format itself stores the style (a JSON string).

The string must be one of `"default"`, `"implicit"`, `"strictImplicit"`, `"instImplicit"`. Decoders enforce membership; unknown styles produce `LeanExprValidationError::UnknownBinderStyle`.

### 3.6 Recursive self-reference

Each of `lean:LeanName`, `lean:LeanLevel`, `lean:LeanLevelList`, `lean:LeanExpr` references itself in some ctor arg's `core:type_name`. This is permitted by the chain's inductive-value validator (D32 §3.3 — `type_name` resolves to the parent inductive's own IRI = recursion). The four inductives also reference each other across the build-up direction (`Sort` carries `LeanLevel`; `LeanLevelList.Cons` carries `LeanLevel` + `LeanLevelList`; `Const` carries both `LeanName` and `LeanLevelList`); these cross-references are valid because the resolution order has `LeanName` and `LeanLevel` declared before `LeanLevelList`, and all three before `LeanExpr` (the ontology layer's commit order is `LeanName → LeanLevel → LeanLevelList → LeanExpr`, per the closure walker's topological order).

## 4. Encoder/decoder semantics

The chain-mirror translator lives in the verification institution's `eigenius-lean` crate (D28 §10.2). Its job: decode a nanoda-parsed `Expr` into a chain-CBOR `lean:LeanExpr` value, and (less commonly) re-encode a chain `lean:LeanExpr` value back into a nanoda `Expr`. Decode runs at commit time on every `LeanProofTerm` to populate the `proposition` field; encode runs only for diagnostics (e.g. re-rendering a chain-stored proposition for a query result).

### 4.1 Decode is structural recursion

For each nanoda `Expr` variant, the translator emits the corresponding chain-CBOR ctor value with structurally-decoded argument fields. The decoding rules are exhaustive (one rule per Expr variant minus `Local`):

```
decode(Expr::Var { dbj_idx, .. })   = ctor Expr.Var { dbj_idx }
decode(Expr::Sort { level, .. })    = ctor Expr.Sort { level := decode_level(level) }
decode(Expr::Const { name, levels, .. }) =
    ctor Expr.Const {
      name := decode_name(name),
      levels := [decode_level(l) for l in levels]
    }
decode(Expr::App { fun, arg, .. })  = ctor Expr.App {
                                        fun := decode(fun),
                                        arg := decode(arg)
                                      }
decode(Expr::Pi { binder_name, binder_style, binder_type, body, .. }) =
    ctor Expr.Pi {
      binder_name := decode_name(binder_name),
      binder_style := decode_binder_style(binder_style),
      binder_type := decode(binder_type),
      body := decode(body)
    }
decode(Expr::Lambda { ... })  = symmetric to Pi
decode(Expr::Let { binder_name, binder_type, val, body, nondep, .. }) =
    ctor Expr.Let {
      binder_name := decode_name(binder_name),
      binder_type := decode(binder_type),
      val := decode(val),
      body := decode(body),
      nondep := nondep
    }
decode(Expr::Proj { ty_name, idx, structure, .. }) =
    ctor Expr.Proj {
      ty_name := decode_name(ty_name),
      idx := idx as i64,
      structure := decode(structure)
    }
decode(Expr::StringLit { ptr, .. }) = ctor Expr.StringLit { value := ptr.as_str().to_string() }
decode(Expr::NatLit { ptr, .. })    = ctor Expr.NatLit { value := ptr.to_string() }
decode(Expr::Local { .. })          = ERROR (closed terms only — see §3.3)
```

`decode_name` and `decode_level` are the analogous structural recursions over `Name` and `Level`.

`decode_binder_style` maps nanoda's `BinderStyle` enum to the four pinned strings (`default` / `implicit` / `strictImplicit` / `instImplicit`).

A `Local` variant in the input raises `ChainMirrorError::UnexpectedLocal` with the path to the offending sub-term — useful diagnostic but should never fire on real export bytes (lean4export never emits Locals; nanoda parses to closed terms).

### 4.2 Sharing — flatten via DAG-to-tree

nanoda's parsed `Expr` is hash-cons'd: `App(f, x)` and `App(f, y)` may share `f` as a single allocation. When decode runs, structurally-identical sub-terms are visited once per occurrence (the recursive descent doesn't observe sharing) and each occurrence emits its own chain-CBOR value.

This is a deliberate flattening. The chain inductive value is a *tree*, not a DAG. Two practical consequences:

- **Proposition size on the chain is bounded by the tree size, not the DAG size.** A proposition that uses `Nat` 30 times has 30 copies of `Const "Nat"` in the chain value. This is fine because propositions are small (the whole proposition for any practical proposition is bounded — Mathlib's largest propositions are thousands of nodes, not millions).
- **Equality of two chain `lean:LeanExpr` values is structural equality**, which corresponds to nanoda's `Expr` equality up to the same hash-consing relation. Two chain-mirrored propositions are equal iff the underlying Lean Exprs were structurally equal — exactly the relation EigenQL queries want.

The DAG-vs-tree question is independent of soundness. Verification uses the *verbatim proof bytes*, parsed via nanoda's DAG. The chain-mirrored proposition is a tree shadow; if it's wrong the verdict is still correct (the bytes carry the load), the chain just queries against a wrong shape. The decoder's correctness is enforced by §4.4's golden-file tests.

### 4.3 Encode (reverse direction)

Encode is the structural inverse of decode. It rebuilds a nanoda `Expr` (via `TcCtx::mk_*` constructors) from a chain `lean:LeanExpr` value. Used only for diagnostics — `LeanProofTerm.proposition` queries that want to render to Lean source go through encode. Not on the verification path.

Encode is total over the v1-supported `lean:LeanExpr` shape: every chain value produces a nanoda `Expr` (modulo the `Local`-absence guarantee — the chain mirror has no `Local` ctor, so no input shape can mention one).

### 4.4 Round-trip discipline

`decode ∘ encode = id` on chain `lean:LeanExpr` values, modulo structural equality on the tree shape. `encode ∘ decode = id_struct` on nanoda `Expr` trees, where `id_struct` is the identity-up-to-sharing-collapse: a DAG-shared sub-term in the input is split into per-occurrence copies in the chain value, then rebuilt as per-occurrence allocations in the output (the resulting nanoda `Expr` will hash-cons identically by structural equality, but the allocation profile may differ from the input's).

The release pipeline runs golden-file round-trip tests:

1. A hand-curated set of canonical propositions (e.g. `∀ n : Nat, n + 0 = n`, `∀ p : Patient, p.weight ≥ 0`, …) is encoded to chain values via the translator.
2. The chain values are checked against committed goldens (byte-equal — same decode rules produce the same CBOR encoding).
3. Each value is then decoded back via encode → checked for structural equality against the source Lean expression (re-parsed via nanoda for comparison).

A translator that fails any golden is non-conformant.

## 5. Use sites

### 5.1 `LeanProofTerm.proposition`

The primary consumer. D28 §6.3's `LeanProofTerm` resource carries `proposition: lean:LeanExpr`, populated at commit time by the institution's `query` handler.

Decode flow (commit time):

1. The institution's `query` handler receives the just-committed `LeanProofTerm`.
2. It runs nanoda on `proof_term_bytes` to validate the proof.
3. It locates the target Theorem (by `target_declaration` name) in nanoda's parsed environment.
4. It reads the Theorem's *type* (the proposition).
5. It runs the §4.1 translator on that type, producing a `lean:LeanExpr` value.
6. The value is stored as the resource's `proposition` field (the resource's committed bytes are amended; this is an institution-driven amendment akin to D14's epistemic stamping).

Encode flow (query time):

1. An EigenQL FIBER query references a `LeanProofTerm`'s `proposition`.
2. The query evaluator returns the chain-mirrored value structurally.
3. Optional: the orchestrator's diagnostic surface renders the value back to Lean source via §4.3's encode for human display.

### 5.2 Cross-institution Comorphisms (D28 §3.4 future work)

The D27 §6.2 Lean ↔ IntervalArithmetic bridge wants to translate `(FormulaTerm, IntervalRepr, IntervalRepr)` triples into Lean proof obligations. The "Lean proof obligation" side is a `lean:LeanExpr` value asserting the bound. The bridge's middle Component (`m` in D14's `(s, m, t)` triple) is a EigenTT function from the Julia-side triple to a chain `lean:LeanExpr` value — it packages the FormulaTerm + bounds into Lean Prop syntax. No bespoke conversion code in the Comorphism: it's a chain-typed EigenTT term consuming `formulas:FormulaTerm` and producing `lean:LeanExpr`.

This is why D40 exists. Without a chain-mirrored Lean expression form, the Comorphism would have to emit Lean source as bytes, the Lean side would have to parse + validate the bytes at reify time, and the audit chain would have a bytes-shaped gap. With D40, the Comorphism produces a structured chain value, the validator type-checks it at commit, and the audit chain stays graph-internal.

### 5.3 EigenQL queries on propositions

Once `lean:LeanExpr` values are on the chain, EigenQL patterns can refer to them:

```eigenql
USING "urn:eigenius:lean:LeanProofTerm"
USING "urn:eigenius:lean:LeanExpr"

MATCH LeanProofTerm(?p) {
    "urn:eigenius:lean:proposition": ?prop,
    "urn:eigenius:lean:eigenius_claim_iri": ?claim
}
WHERE ?prop matches Pi { binder_type: Const { name := <"Nat"> }, body: _ }
RETURN [] { proof: ?p, claim: ?claim }
```

(EigenQL's pattern-match syntax over inductive values is sketched here; the actual surface lands as part of Phase 20a's notebook tooling.)

## 6. Version discipline

`lean4export`'s output format has a semver (3.1.x at time of writing; nanoda's `check_semver` accepts `>=3.1.0, <3.2.0`). D40 v1 targets that range. Version-bump discipline:

- **lean4export 3.1.x → 3.2.x (minor bump).** Additive features (new ctor variants, new Expr fields) require a D40 minor bump to extend the chain inductives. Until the bump lands, decode errors out cleanly on the new variants (the translator's match is exhaustive). Existing chain values remain valid.
- **lean4export 3.x → 4.x (major bump).** Breaking changes (ctor removed, ctor argument shape changes). D40 majors with it. Existing chain values become *historically* valid (against their `lean_export_version` pin on `LeanProofTerm`) but new commits use the new shape. The `lean_export_version` field on `LeanProofTerm` (added in this version) is what pins the spec.
- **D40 spec patches.** Clarifications that don't affect produced values. Existing chain values remain valid.

The `LeanProofTerm` resource (D28 §6.3) gains a `lean_export_version: string` field carrying the lean4export semver. The institution rejects commits whose declared version is outside the spec's accepted range (currently `>=3.1.0, <3.2.0`). Re-checking historical commits against the same lean4export they were created with is part of the closed-audit-chain property.

## 7. Determinism contract

Translator output is deterministic: same input nanoda `Expr` → same chain CBOR bytes. Sources of determinism:

- The §4.1 decoding rules are pure — no allocation order, hash-cons identity, or stable-pointer comparison surfaces in the output. The chain CBOR encodes structurally.
- Eigon-CBOR encoding is canonical (sorted property keys via the standard Eigon serialiser; arrays in source order; primitive encodings shortest-form).
- `BinderStyle` rendering follows the pinned mapping (`default` / `implicit` / `strictImplicit` / `instImplicit`).
- `NatLit` digits are decimal with no leading zeros and no sign.

The release pipeline runs §4.4's golden-file tests + a randomised round-trip test (a fuzz-style harness that generates synthetic propositions, encodes, decodes, asserts structural equality). Hash-stability of the resulting `LeanProofTerm.proposition` field across two independent translator runs is part of v1 conformance.

## 8. Decisions

Three were carried as open questions through the design pass and are now settled.

### 8.1 Omit `Local` from `lean:LeanExpr` (settled)

Closed terms only. Adding `Local` would require encoding `FVarId` (nanoda's free-variable identifier — a `DbjLevel(u16)` or `Unique(u32)` — both shapes have natural chain primitives) and committing to never-fires ceremony.

**Trigger to revisit:** a use case for proof-state inspection where open propositions cross the institution boundary. Likely never in v1; a research follow-on.

### 8.2 `nondep` flag stays as a chain `core:boolean` field (settled)

Lean's `Let` has a `nondep` flag distinguishing dependent let-bindings (whose body's type depends on the let-bound value) from non-dependent ones. nanoda parses + preserves it. The flag is not load-bearing for soundness (nanoda's checker handles both shapes the same way) but it survives in the export format and round-trips through encode/decode. Carrying it on the chain matches the verbatim-bytes representation.

**Trigger to revisit:** lean4export drops the flag, or a chain-side simplification moves to inferring dependence structurally.

### 8.3 Single ontology layer per institution (settled)

The four inductives (`lean:LeanName`, `lean:LeanLevel`, `lean:LeanLevelList`, `lean:LeanExpr`) commit to a dedicated `ontologies/lean/lean-expressions.eigon.json` ontology layer (Phase 20a.2). The layer sits in the kernel's bootstrap chain (between `formulas` and `notebook` per [`kernel/src/bootstrap/mod.rs`](../../kernel/src/bootstrap/mod.rs)) so chain-resident `lean:LeanExpr` values type-check at commit time without requiring the Lean institution itself to be registered. The institution declaration (`LeanProofTerm`, `LeanEnvironment`, etc., per D28 §10.3) lives in a separate `ontologies/lean/lean-institution.eigon.json` layer that's *not* bootstrap-loaded — it's applied when the Lean institution is registered.

**Trigger to revisit:** if a non-Lean institution wants to consume `lean:LeanExpr` independently, the layer might split out. No such consumer exists in v1.

## 9. References

- [nanoda_lib repository](https://github.com/ammkrn/nanoda_lib) — the Rust Lean term checker; vendored at [references/nanoda_lib/](../../references/nanoda_lib/).
- [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/) (Chris Bailey) — the canonical specification of Lean's kernel-level semantics, the basis for nanoda's design.
- [lean4export](https://github.com/leanprover/lean4export) — the official Lean export tool whose JSON output nanoda parses.
- [D14 — Institution Realisation](d14-institution-realisation.md) — the institution protocol that registers Lean as a verification institution.
- [D19 — EigenTT Inductive Types](d19-inductive-types.md) — the kernel mechanism the chain inductives sit on.
- [D26 — Runtime Substrate](d26-runtime-substrate.md) — the substrate that hosts Lean's authoring side.
- [D28 — Lean 4 as Verification Institution](d28-lean-4-as-institution.md) — the parent integration spec.
- [D30 — Eigon → Lean Faithful Translation](d30-eigon-to-lean-faithful-translation.md) — the sibling spec for the EigonFFI mirror.
- [D32 — Chain-Mirrored EigenTT Inductives + FormulaTerm](d32-chain-mirrored-mini-tt-inductives.md) — the structural precedent this design follows.
