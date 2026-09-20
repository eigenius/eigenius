# D94 — Exact numerics: `core:bigint` and `core:rational`

**Written** `2026-09-20`, against `main` at `1efd6bb`. **First draft.** Consumers: D86 §4 (the `Rat`
pivot) and D93 (conversion factors and SI prefixes). Both are blocked on the same decision, which is
why it is neither's to make.

## The gap

The kernel has two numeric carriers and neither is exact at scale. `PrimitiveType` is
`String | Iri | Integer | Float | Boolean | Json` (`kernel/src/nbe/term.rs:402`); `Exp::LitInt` is
`i64` and `Exp::LitFloat` is `f64`. There is no arbitrary-precision integer and no rational
anywhere in `kernel/src/`.

That is not a theoretical shortfall. **The SI's own prefix set does not fit.** `u64` tops out at
18,446,744,073,709,551,615 ≈ 1.845 × 10¹⁹:

| prefix | factor | fits `u64` |
|---|---|---|
| exa | 10¹⁸ | yes |
| zetta | 10²¹ | **no** |
| yotta | 10²⁴ | **no** |
| ronna | 10²⁷ | **no** |
| quetta | 10³⁰ | **no** |

The four largest prefixes are recent: ronna, ronto, quetta and quecto are CGPM Resolution 3 (2022),
not part of the 9th edition's 2019 printing, which had 20.

and symmetrically for zepto, yocto, ronto and quecto, whose exact rational forms need denominators
of 10²¹ through 10³⁰. Eight of the twenty-four prefixes are unrepresentable.

Conversion factors hit the same wall. 1 eV = 1.602176634 × 10⁻¹⁹ J **exactly**, since the 2019 SI
fixes the elementary charge. As a rational that is 1602176634/10²⁸, which **reduces** to
801088317/(5 × 10²⁷) — the canonical form this document mandates, and still far beyond `u64`. The
unreduced form is given first only because it is how the decimal transcribes; a stored literal must
be the reduced one.

And D86's pivot needs more again. An exact binary64 rational has a power-of-two denominator: `0.05`
is 3602879701896397/2⁵⁶, which fits — but subnormals run to 2¹⁰⁷⁴, a **324-digit** denominator.

## The asymmetry that makes this urgent

**The checker we host can already express values EigenTT cannot.** `nanoda_lib` represents Lean
`Nat` literals as `BigUint`, and Lean's `Rat` is `structure Rat where mk' :: num : Int; den : Nat`
at arbitrary precision (D86 §4 quotes it).

**Whether that is urgent depends on a reading of D86 that D86 does not settle.** Under the reading
its §4 describes — `core:float` *mapping to* an exact rational at externalisation, via
`BigRational::from_float` in the translator — the chain holds a `LitFloat` and the large rational
exists only in the emitted Lean term. Nothing is then unrepresentable and there is no asymmetry.
The asymmetry arises only under the stronger reading, where the chain's own numeric carrier becomes
exact — which D86 gestures at ("the argument that chose `Float` over an exact type is hollow")
without taking.

D93 forces the question independently: exact conversion factors and prefixes are chain data, not
translator output, and eight prefixes exceed `u64`. So the requirement stands on D93's evidence.
It should not lean on D86's, and the bound below must be justified from a requirement that is
actually established rather than from 2¹⁰⁷⁴.

A host type theory narrower than the checker it hosts is backwards, and the comorphism is where it
would surface — as a value that fails to externalise rather than a claim that fails to hold.

## The decision

**`core:bigint` and `core:rational` become kernel primitives**, owned here rather than by either
consumer.

- `core:bigint` — a signed arbitrary-precision integer.
- `core:rational` — a pair of bigints in **canonical form**: denominator strictly positive, `gcd`
  of numerator and denominator equal to 1, zero represented as 0/1.

Canonical form is the point. With it, equality of rationals is **structural** — the same discipline
D86 chose for float literals (canonical form in the term, arithmetic out of the checking loop) and
D93 chose for unit normal forms. Three carriers, one rule.

`num-bigint` is already a dependency in the tree (`crates/eigenius-lean/Cargo.toml`), and
`num-rational` is present transitively, so the implementation borrows rather than mints.

## Literals are new variants, not a widened `LitInt`

`Exp::LitInt` stays `i64`. The exact carriers get their own variants.

**Not because widening would force a reseed** — it would, but so does this document: new primitives
and new core declarations move the bootstrap either way, so that cost does not discriminate between
the options. The reasons are these.

**The requirement is exact rationals, and a bigint is their component rather than a replacement for
ordinary integers.** Every large magnitude identified by a consumer appears as a numerator or a
denominator: prefixes as powers of ten, conversion factors, binary64 denominators. A general integer
literal in this system is an arity, an index, a count, a universe level, a string-length bound.
Those are machine-sized by their nature, and nothing has been shown to need one above `i64`.

**Widening propagates into `Constraint`, where it is wrong.** `MinValue(i64)` and `MaxValue(i64)`
compare against `LitInt`, so a widened literal leaves them either heterogeneous or widened too. And
`MinLength(i64)` / `MaxLength(i64)` are *string lengths* — a bigint bound on a string's length is
meaningless. The widening does not stop at the literal.

**The two carriers are a real distinction, not redundancy.** `core:integer` and `core:bigint` make
different promises, and a slot should say which it means: an array index is not an exact numerator
that happens to be small. What should hold is **admissibility in one direction** — a machine integer is a bigint, so `LitInt`
where `core:bigint` is expected should check.

There is no existing mechanism to cite for this. `PrimitiveType` has no `admits`; the one carrier
relation that exists is `LitString` checking against `PrimitiveType::Iri` in **check mode only**
(`kernel/src/nbe/check/mod.rs:540-560`), and D88 §3 makes `Iri` a *refinement* — the narrower type
admitted where a declared type asks for it. `LitInt : core:bigint` is the opposite direction, so it
is a new rule and must be argued as one rather than by an analogy that inverts.

## Bounded, because a checker must terminate predictably

Arbitrary precision in a *type checker* is an unbounded-resource surface. A term can demand
10^1000000 as easily as 10^30, and the kernel would compute it. Against a chain that accepts
authored terms, that is a denial-of-service vector in the trusted computing base.

v1 therefore carries a **declared bound on magnitude** — a maximum bit-width for a bigint, checked
when a literal is admitted, refusing rather than truncating.

**This document does not yet give the number, and an earlier draft claimed it did.** The number
depends on two things it has not settled: which requirement is actually established (see the
reading of D86 above — 2¹⁰⁷⁴ may not be justified), and whether magnitudes are computed on at all,
since a literal bound constrains nothing if a multiplication can double a value's width. A worked
composition makes the point: a binary64-derived denominator times a 10⁻³⁰ prefix is about 1175
bits, and sizing consumers one at a time misses it.

Refusing loudly is the required behaviour. A silently truncated exact value is worse than no exact
value, because it looks like the thing it is not.

**The bound only bounds anything if magnitudes are not computed on.** A literal bound does not
constrain a multiplication that doubles a value's width. See "What is the magnitude bound, and does
it bound anything?" below — it is the same question as how much arithmetic the kernel does, and the
two must be settled together.

## Why not the alternatives

**A decimal type instead of a rational.** Exact binary64 values *are* finitely representable in
decimal, so D86 would be served. D93 would not: conversion factors like 1/3 and the general
commensurability ratio are not finite decimals. A rational subsumes decimal; the reverse does not
hold.

**Wider fixed-width floats (f128).** Still floating, still inexact, and it reproduces every D86
problem at a larger size — non-reducing literals, the normalisation workaround, the version gate.
The requirement is exactness, not range.

**Keep `i64` and have callers scale.** This is what `core:float` does today with units, and it is
the defect D93 exists to remove: a scale that lives in the caller's head is not checked.

## Open questions

### Is `core:bigint` separately needed?

A rational with denominator 1 is an integer, so a single `core:rational` carrier could serve both,
with `core:bigint` as a *refinement* checking `den == 1` — the relationship `PrimitiveType::Iri` has
to `String` (a refinement sharing a carrier, not a separate one, D88 §3).

Two things argue against collapsing them:

- **A rational is built from bigints.** Numerator and denominator are integers of arbitrary size, so
  the concept is required even where the literal carrier is not.
- **The Lean comorphism needs them separately.** Lean's `Rat` is `num : Int, den : Nat` (D86 §4).
  Externalising a rational means producing an `Int` and a `Nat`, not a `Rat` atom, so integer-shaped
  exact values cross the boundary whether or not the chain has a carrier for them.

What would decide it: whether any slot wants an exact integer that is not part of a rational.
Nothing identified so far does — every large magnitude is a numerator or a denominator.

### How much arithmetic must the kernel do?

The draft assumed "literals only, no arithmetic". **That is already false, in two places.**

**Canonical form requires `gcd`.** A rational is canonical when numerator and denominator are
coprime, and *verifying* coprimality is computing the gcd — there is no cheaper check. The
alternative is to trust the authored form, which is unsound: a non-reduced rational would compare
unequal to its reduced twin, and structural equality is the whole reason for canonical form. So the
kernel computes one gcd per rational literal at admission. This is exactly Lean's own `Rat`
invariant, whose `reduced` field is discharged the same way, and D86 §4 records nanoda accelerating
gcd for precisely this.

**D93's unit normalisation requires rational addition.** Multiplying units adds exponents:
`m * m → m²` needs 1 + 1, `m * m⁻¹ → 1` needs 1 + (−1), and with rational exponents
`m^(1/2) * m^(1/2) → m` needs ½ + ½. The normaliser lives in the kernel primitive, so rational
addition is in the trusted surface whether or not general arithmetic is.

**This suggests a distinction the draft did not make**, and it may be the resolution:

| | magnitude rationals | exponent rationals |
|---|---|---|
| size | large — 2¹⁰⁷⁴, 10³⁰ | tiny — ½, ⅓, 2, −1 |
| operations needed | equality, and gcd once at admission | add, subtract, negate, **multiply by a scalar**, and **gcd** |
| bound | generous, refuses above it | very small; nothing needs `m^(10²⁰)` |

**The exponent column is larger than an earlier draft claimed.** Addition alone does not cover it:

- **Scalar multiplication.** Expanding a derived unit under a power multiplies its whole exponent
  vector — `N²` → `kg² m² s⁻⁴`. `√Hz` is `(s⁻¹)^(1/2)` → `s^(−1/2)`, which is this document's own
  first motivating row.
- **gcd.** `a/b + c/d = (ad+cb)/(bd)` is not reduced, and this document's own argument — verifying
  coprimality has no cheaper check than computing the gcd — applies identically to exponents.
- **Integer multiplication**, since `ad`, `cb` and `bd` are products.

So the arithmetic-heavy carrier needs a small rational *field*, not an additive group. It remains
bounded, which is the property that matters.

If the two are separated, the unbounded carrier needs almost no arithmetic and the arithmetic-heavy
one needs almost no range. That is a much smaller trusted surface than one general-purpose exact
rational with full arithmetic.

### What is the magnitude bound, and does it bound anything?

The known requirements are modest: 2¹⁰⁷⁴ for binary64 subnormals is the largest justified, which is
1075 bits; quetta at 10³⁰ is about 100 bits. A bound of a few thousand bits clears both with room.

But **a bound on literals does not bound computation.** Multiplying two 1000-bit values yields 2000
bits. So the bound is only a real limit if the kernel does no size-increasing arithmetic on
magnitudes — which is the case if the section above holds, and is not if general arithmetic is
admitted later. The two questions are one question.

Shape also matters: a bit-width bound on a literal is checked once and cheaply. A fuel or
term-size budget would bound computation too, at the cost of being a much larger change to how the
checker accounts for work.

### ESL surface syntax

How an author writes an exact value, and — given canonical form — **who reduces it**.

If the surface is a division (`1602176634/10^28`, reducing to `801088317/(5×10^27)`), the elaborator
must reduce before the term is
built, because a division in the term is arithmetic in the checking loop. That is D86 §4's lesson
about emitting `Rat.mk'` rather than `HDiv.hDiv`, and D93's about canonical unit spines: normalise
in the bridge, compare structurally in the kernel.

So the elaborator reduces and the kernel verifies (one gcd). A decimal surface that elaborates to
the exact rational — `0.05` meaning 1/20 rather than the binary64 value — would be convenient and is
a trap: D86 §4 is explicit that a stored `0.05` is the binary64 value `0.05000000000000000277…`, and
a surface that silently means something else reintroduces the delta that section exists to prevent.
If both are wanted they need distinct syntax.

## Scope

**In.** `core:bigint` and `core:rational` as primitives; canonical form for rationals; new `Exp`
literal variants; refusal behaviour above a bound; the chain-level type declarations.

**In, and unspecified — this document must not be implemented until they are.** The bound's actual
value; whether exponent rationals are a separate bounded carrier from magnitude rationals (which
changes whether "In" names two primitives or three); and the chain-side surface every new literal
needs — a new `eigentt:Term` constructor, encode/decode arms in the D47 mirror
(`kernel/src/program/eigentt_type_mirror.rs`), a carrier in `ontology::Value` (whose `Integer` is
documented as the 53-bit safe range, so an exact literal has no home today), the D1 Eigon-JSON form,
and the ESL printer and compiler arms.

**In, and only discovered while elaborating the open questions:** `gcd` at literal admission, since
verifying canonical form has no cheaper check; and rational addition on D93's unit exponents, since
normalising `m^(1/2) * m^(1/2)` to `m` is exponent arithmetic. The draft's original scope said
kernel-side arithmetic was out unless a consumer needed one — two do.

**Out of v1.** General size-increasing arithmetic on magnitudes (multiplication, division), which is
what would make the magnitude bound meaningless — note this exempts the exponent arithmetic admitted
above, which is size-increasing but bounded, and the exemption is deliberate; a decimal surface type; arbitrary-precision floats;
any change to `LitInt` or `LitFloat`.
