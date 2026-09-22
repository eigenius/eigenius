# D94 implementation plan — `core:bigint` and `core:rational`

Ordered slices for D94 ("Exact numerics"). D93 depends on this work for conversion factors and for
rational arithmetic on its unit exponents, so this lands first.

The seam that shapes the order is the **bootstrap**: adding a `core:` type declaration changes
`ontologies/core/core-ontology.json`, which forces a database reseed (~20 min, and 80 GB of headroom
on the machine this was last run on).

**Decided: one reseed, after both D94 and D93.** So the chain surface is not the last slice of D94 —
it is half of a joint slice shared with D93, and D94's Rust-side work (slices 1–2) lands first with
no chain change at all. Beyond saving a cycle, this forces one question to be answered once rather
than twice: D94 lowers a composite magnitude to `Value::Embedded` and D93's `Unit` carries three
exponent vectors, and the embedded shape for those has not been written down. Designing both codec
walks together settles it.

## Blast radius, measured

`Exp::LitInt` is matched in **19 kernel files**. The concentration is what matters:

| File | Sites |
|---|---|
| `kernel/src/program/eigentt_type_mirror.rs` | 9 |
| `kernel/src/nbe/eval/mod.rs` | 5 |
| `kernel/src/esl/compile.rs` | 5 |
| `kernel/src/nbe/term.rs` | 4 |
| `kernel/src/nbe/eval/marshal.rs` | 4 |
| `kernel/src/nbe/eval/iota.rs` | 4 |
| `kernel/src/esl/print.rs` | 4 |
| `kernel/src/esl/ast.rs` | 3 |
| 11 others | 1–2 each |

**The lowering decision keeps this from being much larger.** D94 carries a rational as
`Value::String` rather than a new `Value` variant, so the `ontology::Value` side is untouched —
`Value::Integer` alone has 18 sites in `query/functions.rs`, 14 in `layer/merge/resolve.rs`, 11 in
`esl/compile.rs`, 10 each in `validation/mod.rs`, `task/sweep_registry.rs` and `eigon_cbor.rs`. None
of those are in scope. Confirm this holds before starting; it is the single largest cost driver.

## Slice 1 — the numeric core, no chain change

New module `kernel/src/numeric/`, depending on `num-bigint` (`0.4.4` is already vetted in-tree at
`crates/eigenius-lean/Cargo.toml`; the kernel has no bigint dependency today).

- A `Rational` newtype whose constructor **establishes** canonical form: reduced by gcd, `den > 0`,
  sign on the numerator, `den == 1` for integers.
- The 4096-bit bound, refusing rather than truncating. Refusal is an error type, not a panic.
- Parse and print the canonical decimal grammar `'-'? uint ('/' uint)?`, where `uint` is `'0'` or
  `[1-9][0-9]*`.

Tests: round-trip through the string form; two constructions of one value compare equal; a
non-canonical string is rejected rather than normalised on read; the bound refuses at 4097 bits and
admits at 4096.

No `Exp` change, no chain change, no reseed. Fully isolated.

## Slice 2 — the `Exp` carrier

- `Exp::LitRat(Rational)` beside `LitInt`/`LitFloat` in `kernel/src/nbe/term.rs:188-203`.
- `PrimitiveType::Rational` and `PrimitiveType::BigInt` at `term.rs:402`.
- Match arms across the 19 files. Most are one line: a literal is inert under `subst`, `unify`,
  `readback`, `positivity` and `iota`.
- Definitional equality: canonical form makes two rationals equal iff their components are equal, so
  `conv` needs no arithmetic. This is the property slice 1 exists to guarantee.

Still no chain change.

## Slice 3 — D93's unit algebra (Rust-side, no chain change)

Not D94's work, but it belongs in this order: D93's exponent canonicalisation needs slice 1's
rational arithmetic (normalising `m^(1/2) * m^(1/2)` to `m` is exponent addition), and the joint
chain slice cannot start until both algebras exist. Build it against slice 1 directly.

## Slice 4 — the joint chain surface (D94 + D93), then one reseed

D94's half:

- `core:bigint` and `core:rational` declarations in `ontologies/core/core-ontology.json`.
- A new `eigentt:Term` constructor and its arms in `kernel/src/program/eigentt_type_mirror.rs`
  (9 sites — the heaviest single file).
- Eigon-JSON: emit the canonical decimal string; the decoder keys on the slot's declared
  `data_type`. `kernel/src/ontology/eigon_json.rs`.
- CBOR needs **no change**: `Value::String` already maps to `Text` and back
  (`eigon_cbor.rs:203`, `:370`).
- ESL: the `r` suffix in `kernel/src/esl/lexer.rs` and `parser.rs`, and the printer arm in
  `print.rs`. The suffix is surface-only and does not reach the chain.
D93's half lands in the same commit: the `Unit` primitive's `core:` declaration, its
`eigentt:Term` constructor and mirror arms, its `Value::Embedded` form, and the ESL surface for unit
literals.

**Then one reseed**, exercising both.

**What can still be tested before it.** The codec round-trip does not need the bootstrap change:
`eigon_cbor.rs`'s existing tests construct `Resource`s in Rust directly (`Value::String("Alice")`
and friends, around `:422-455`) rather than loading an ontology. So slice 2 can prove
`Rational → Value::String → Text → Value::String → Rational` end to end. Only the part that
discriminates by **declared `data_type`** needs the chain, and that is the narrow thing the reseed
buys. This matters because the CBOR and Eigon-JSON boundaries are exactly where D84 §5's variant
failures showed up — they are worth exercising early, not at the end.

## Slice 5 — validation

- `kernel/src/validation/rules/eigentt_value.rs` — the rule admitting the new literal.
- **Check the `Constraint` interaction before writing it.** `MinLength` is a string length, and a
  rational is carried as a string, so a length constraint on a `core:rational` slot would measure
  the wrong thing. Decide whether constraints are rejected on rational slots or reinterpreted.

## Decisions the implementation forces

1. **One carrier or two.** D94 leaves this open: *"What would decide it: whether any slot wants an
   exact integer that is not part of a rational. Nothing identified so far does."* The cheaper path
   is one `Exp::LitRat` carrier with `core:bigint` as a **refinement** checking `den == 1` — the
   relationship `PrimitiveType::Iri` has to `String` (D88 §3), which D94 itself cites as the
   precedent. That gives two declared chain types over one carrier, satisfying D94's "two chain
   primitives" without two `Exp` variants. Decide it at the top of slice 2.
2. **Where the bound is enforced.** Slice 1 puts it in the constructor, so no unbounded value can
   exist. The alternative — checking at admission only — leaves a window. Prefer the constructor.
3. **What `LitInt` does now.** It stays: `core:integer` is a distinct declared type with a 53-bit
   range, and nothing in D94 retires it.

## Verify before starting

- That `Value::String` really is the lowering carrier and no `Value` variant is needed. The whole
  cost estimate rests on it.
- What an embedded magnitude looks like concretely — D94 says a magnitude is composite and lowers to
  `Value::Embedded`, but D93's `Unit` carries three exponent vectors and the embedded shape for
  those has not been written down.
