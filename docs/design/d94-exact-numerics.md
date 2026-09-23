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

**The number is 4096 bits**, justified under "The magnitude bound" below. It is set for
termination rather than precision, and it is meaningful only because magnitudes are never computed
on — a literal bound constrains nothing if a multiplication can double a width.

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

## Decided while elaborating

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

### How much arithmetic must the kernel do? — decided

**Why a rational and not a decimal — the question that decides whether gcd is needed at all.**
Every terminating decimal is a rational with a `2^a·5^b` denominator, so a decimal carrier
(arbitrary-precision significand, base-10 exponent) would be more compact and **canonical without
arithmetic**: strip trailing zeros, and `0.05`, `5/100` and `1/20` all become `(5, −2)` by
inspection. Its coverage is wide — every v1 conversion factor terminates (eV→J, lb→kg, hour, au,
every prefix), as does every binary64 value, since 2 divides 10. Measurements, thresholds and D86's
exact float values would all be decimals.

**D93's magnitude decision is what rules it out.** The angle factors do not terminate: `37/180`
(180 = 2²·3²·5) and `1/32400` (32400 = 2⁴·3⁴·5²) both carry a factor of 3. So admitting the
transcendental category — angles, the parsec, atomic units — requires general rationals, and general
rationals require reduction. Had that decision gone the other way, this document would need **no
kernel arithmetic whatsoever**.

The cost is one gcd per literal, and it is the price of `37π/180`. Lean's `Rat` pays the same price
for the same reason. It is also not an angle-only quirk: `torr` is `101325/760`, which does not
terminate either — any unit defined by a ratio rather than a decimal lands here.

**`gcd` at admission, and nothing else.** A rational is canonical when numerator and denominator are
coprime, and verifying coprimality *is* computing the gcd — there is no cheaper check, and trusting
the authored form is unsound, since a non-reduced rational would compare unequal to its reduced
twin. So one gcd per literal at admission. This is Lean's own `Rat` invariant, discharged the same
way, and D86 §4 records nanoda accelerating gcd for exactly this.

**Exponent arithmetic is not this document's.** An earlier draft put D93's unit-exponent arithmetic
here, on the grounds that the kernel needs it. It does — but not on `core:rational`. D93's decisions
separated the two cleanly:

| | magnitude coefficient | the three exponent vectors |
|---|---|---|
| carries | `10³⁰`, `10⁻²⁸`, composed values | `1`, `2`, `−3`, `½`, `−⅓` |
| needs | arbitrary precision | a **small fixed width** — 16 bits of numerator and denominator allows `m^32767` |
| operations | equality; gcd once at admission | add, subtract, scalar-multiply, gcd |
| where | `core:rational`, this document | inside the `Unit` primitive, D93 |

So exponents never touch `core:bigint`. They are an internal representation of the `Unit` primitive
with their own tiny bound, and the arithmetic on them belongs to D93's normaliser. That answers the
"two carriers or three" question: **two chain primitives**, and exponents are not one of them.

### The magnitude bound — decided

**4096 bits**, checked when a literal is admitted, refusing rather than truncating.

The largest identified requirement is a binary64-derived denominator (2¹⁰⁷⁴) composed with a quecto
prefix (10⁻³⁰) — about **1175 bits**. Four thousand and ninety-six clears that by more than triple,
covers every prefix and conversion factor with room, and still refuses `10^1000000` long before
anything threatens termination. The bound is for **termination, not precision**: it exists so an
authored term cannot ask the checker for unbounded work, and a value that needs more than 4096 bits
is far likelier to be an attack or a bug than a measurement.

**The bound is meaningful because magnitudes are never computed on.** A literal bound constrains
nothing if a multiplication can double a width — and this document puts size-increasing magnitude
arithmetic out of v1 precisely so the bound holds. D93's base-units-only decision is what makes that
affordable: conversion happens at ingest, outside the kernel, so the kernel only ever compares.

## Decided: carrier disambiguation and lowering

### Is a numeric literal polymorphic in its carrier? — decided: no

**Not open: who reduces.** The elaborator reduces and the kernel verifies with one gcd. A division
left in the term is arithmetic in the checking loop, which is D86 §4's lesson about emitting
`Rat.mk'` rather than `HDiv.hDiv` and D93's about canonical forms — normalise in the bridge, compare
structurally in the kernel.

**Not open either: the radix.** IEEE 854 parameterised the radix because floating point
*approximates*, and the radix fixes the lattice of representable values — `1/10` is on the decimal
lattice and off the binary one. Exact rationals have no lattice: every rational is representable, so
`0.05`, `5×10⁻²` and `1/20` are three spellings of one value rather than three approximations. Radix
has no semantic role here, only a surface one.

**The question is disambiguation.** Given that the spellings agree, how does an author say *this
literal is exact* rather than *this literal is a binary64 approximation*? Three answers:

- **Type-directed** — one spelling, the expected type resolves the carrier. Lean does this, writing
  `(0.05 : Rat)` and `(0.05 : Float)` identically via `OfScientific`.
- **A distinct marker** — `0.05r` or similar. Verbose; makes the carrier visible without knowing the
  slot's type.
- **No decimal surface for exact values** — write `1602176634/10^28`. Unambiguous and unreadable.

**There is an in-codebase precedent for the first, and it does not transfer.** D88 §3 makes `Iri` a
check-mode-only refinement of `String`: *"`LitString` INFERS to `String`, never to this: a bare
literal cannot know which it is meant to be. `Iri` is reachable only in CHECK mode, where a declared
type asks for it."* Same shape, and safe there — because `"urn:x"` as a `String` and as an `Iri` is
the **same value**; the refinement adds a check, not a denotation.

`0.05` as a float and as a rational are **different numbers**: `3602879701896397/2⁵⁶`, which is
`0.05000000000000000277…`, versus `1/20`. So the polymorphic reading carries a hazard the precedent
does not:

- two occurrences of `0.05` in one document denote different numbers depending on their slot's type;
- and retyping a slot from `core:float` to `core:rational` changes every literal's value **with no
  textual diff**. D86's pivot is precisely such a retyping, and the manifest pin would record that
  the ontology moved while nothing recorded that the numbers did.

That asymmetry is the argument, and it is why this is a decision rather than an inherited
convention.

**Decided: a distinct marker, as a surface feature.** `0.05` is binary64; `0.05r` is exact `1/20`.
The carrier is visible in the source text without knowing the slot's type, so neither hazard above
can fire: two occurrences of `0.05` denote the same number wherever they appear, and retyping a slot
from `core:float` to `core:rational` leaves every literal's value unchanged — the retyping either
type-errors against the unsuffixed literals or leaves them alone. D86's pivot becomes a diff.

The marker is surface only. It does not reach the chain, because by then the carrier is a declared
`data_type` rather than a spelling.

**Two surface forms, not one — the suffix alone is incomplete.** Only a rational whose reduced
denominator is `2^a · 5^b` has a terminating decimal, so the suffix cannot express `37/180`, which
is this project's own degrees-to-radians coefficient (D93), nor `1/3`. The surface therefore admits:

| Form | Example | Role |
|---|---|---|
| decimal suffix | `0.05r` | what an author writes; exact, so `0.05r` is `1/20` and never the binary64 |
| canonical | `r"37/180"` | always expressible, always reparses; **what the printer emits** |

`r"..."` is lexed before the identifier branch, so a bare `r` is still an identifier, and `5rem`
still lexes as an integer followed by a name. The quoted form refuses a non-canonical spelling
rather than normalising it, for the reason the codec does: two spellings of one value would hash
differently.

**Extending the lexer to accept `37/180r` was rejected.** On seeing `37/180` it would have to decide
literal-versus-division with only a trailing `r` to disambiguate, which is unbounded lookahead past
a `/`.

### Lowering to Eigon-JSON — decided

**An exact value cannot be a JSON number.** `Value::Integer` is documented as *"Signed integer in
the 53-bit safe range"* (`kernel/src/ontology/resource.rs:32-33`), which states the constraint
outright: JSON numbers are read as IEEE doubles by every practical parser, so anything above 2⁵³
does not round-trip. The exact `0.05` is `3602879701896397 / 2⁵⁶`; the numerator (≈3.6×10¹⁵) fits
under 2⁵³ and the denominator (≈7.2×10¹⁶) does not. The eV denominator, 10²⁸, misses by twelve
orders of magnitude.

**Decided: canonical decimal `num/den`, carried as `Value::String` and discriminated by the
property's declared `data_type`.**

```
rational ::= '-'? uint ( '/' uint )?
uint     ::= '0' | [1-9] [0-9]*
```

- reduced — `gcd(num, den) = 1`, established at admission (see "How much arithmetic must the kernel
  do?");
- `den > 0`, with the sign carried on the numerator;
- `den = 1` emits a bare integer, with no `/1`;
- no leading zeros, no `+`, no whitespace.

Canonical form makes string equality value equality, so `values_equal` needs no rational-specific
arm. That is the one thing `ResourceRef` got wrong and D84 §5 records: it "has a special case in
`values_equal` that derived `PartialEq` does not share."

**No new `Value` variant.** D84 §5 withdrew `Value::Json`'s split into `Inductive` /
`InductiveRef` and gives two boundary tests a variant must pass. `Value::Rational` fails both:

1. *The parser cannot produce it.* Discriminating `"1/20"` from the string `"1/20"` requires the
   property's `data_type` — a schema fact — and the parser has no `Layer`, because `bootstrap`
   parses `core-ontology.json` with `parent: None` and that parse creates `core:data_type`.
2. *CBOR cannot preserve it.* `eigon_cbor.rs:203` maps `Value::String → Text` and `:370` maps
   `Text → Value::String`. A rational encoded as text decodes back as a string.

A variant failing both "exist[s] only between `LayerBuilder::build` and the next serialisation."
The house pattern after D84 is `Value::iri`, which returns an ordinary `Value::String` and "makes no
claim a reader could depend on" (`resource.rs:47-56`). Rationals follow it.

**The magnitude is `Value::Embedded`, not a string.** D93's magnitude is `q × Π cᵢ^{eᵢ}` — a
rational coefficient and a constant-exponent vector — so it is composite, and a delimited string
would re-encode structure that Eigon-JSON already has. `Embedded` is anchored on a shape rather than
a schema lookup, which is why D84 kept it: `eigon_cbor.rs:207` and `:397` round-trip it through the
map form. The coefficient inside is a `core:rational` string by the rule above.

**The suffix survives lowering, which is what makes it more than cosmetic.** `0.05` lowers to
`Value::Float(0.05)` and serialises as the JSON number `0.05`; `0.05r` lowers to a `core:rational`
and serialises as `"1/20"`. The two are structurally distinct in the serialised form, so the
distinction is legible to a reader who never saw the ESL source.

### Why not base64 — decided

The codebase already assigns base64 a meaning: opaque bulk bytes in a string-typed slot. From
`crates/eigenius-julia/src/conventions.rs:76-79` — *"Binary content rides through base64 because the
ontology declares the property as `data_type: json`"* — and every call site is a file payload
(`content_base64`, `content_b64`, source-tree archives). A rational is a small semantic scalar, not
binary content.

Base64 is 1.8× more compact asymptotically (1.33 chars/byte against decimal's 2.41), and
bytes→bigint parses linearly where decimal→bigint is superlinear. Both wins are gated on magnitude,
and the crossover sits above every value this document cites:

| value | bits | decimal | base64 + framing |
|---|---|---|---|
| `1/20` | 9 | 4 | ~8 |
| exact `0.05` (`3602879701896397/2⁵⁶`) | 109 | 34 | ~24 |
| eV denominator `10²⁸` | 93 | 29 | ~16 |
| at the 4096-bit bound | 4096 | ~1233 | ~683 |

Base64 also does not reduce the canonicalisation burden. It trades four traps — leading zeros, sign
placement, `den = 1`, reduction — for five: padding, alphabet (`+/` against `-_`), minimal-length
two's complement, sign convention, and num/den framing. The trap it adds matters because the content
hash runs over the serialised resource: a canonicality violation in decimal is visible (`01/20`),
and one in base64 is not (a spurious leading `0x00` byte). When two resources hash differently over
the same rational, decimal shows which digit diverged.

**The condition that would reopen this:** rationals routinely landing within an order of magnitude
of the 4096-bit bound. The size and parse-cost arguments turn on together at that point. The bound
is a refusal threshold, not a typical size, so nothing currently approaches it.

## Scope

**In.** `core:bigint` and `core:rational` as primitives; canonical form for rationals; new `Exp`
literal variants; refusal behaviour above a bound; the chain-level type declarations.

**Two chain primitives, not three.** `core:bigint` and `core:rational` carry magnitude
coefficients. The three exponent vectors D93 defines — dimension, kind, constants — are small fixed
width and internal to the `Unit` primitive; they never touch arbitrary precision, and the arithmetic
on them is D93's.

**Bound: 4096 bits**, refusing rather than truncating.

**Also in: the chain-side surface every new literal needs** — a new `eigentt:Term` constructor,
encode/decode arms in the D47 mirror (`kernel/src/program/eigentt_type_mirror.rs`), a carrier in
`ontology::Value` (whose `Integer` is documented as the 53-bit safe range, so an exact literal has
no home today), the D1 Eigon-JSON form, and the ESL printer and compiler arms.

**In, and only discovered while elaborating:** `gcd` at literal admission, since
verifying canonical form has no cheaper check; and rational addition on D93's unit exponents, since
normalising `m^(1/2) * m^(1/2)` to `m` is exponent arithmetic. The draft's original scope said
kernel-side arithmetic was out unless a consumer needed one — two do.

**Out of v1.** General size-increasing arithmetic on magnitudes (multiplication, division), which is
what would make the bound meaningless. Exponent arithmetic is not an exception to this: it is
size-increasing but bounded at 16 bits, and it lives in D93's `Unit` primitive rather than on
`core:rational`. a decimal surface type; arbitrary-precision floats;
any change to `LitInt` or `LitFloat`.
