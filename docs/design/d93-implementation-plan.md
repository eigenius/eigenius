# D93 implementation plan — units of measure

Ordered slices for the remainder of D93. [`kernel/src/units.rs`](../../kernel/src/units.rs) already
carries the algebra — `Unit`, `Exponent`, `Magnitude`, `BaseDimension`, `Kind`, `Constant`, and 13
tests including D93's three-row kind table. **Nothing outside that file references it.** Everything
below is the work of connecting it.

D94 is done and landed, so exact rationals are available for conversion factors.

## Three decisions block coding

None can be worked around, and each changes what gets written.

1. **The embedded shape for `Unit` and `Magnitude`.** D94's lowering settles that a magnitude is
   composite and lowers to `Value::Embedded`, but not what the fields are. `Unit` carries three
   exponent vectors (dimension ℚ⁷, kind ℚᴷ, and — on a magnitude — constants ℤ^C); `Magnitude`
   carries a rational coefficient beside its constant exponents. Flagged as "verify before starting"
   in the D94 plan and still open.
2. **`931g`.** D93 states v1 "must do one of two things, and it must say which": admit standard
   gravity into the vocabulary, or refuse the numeral/unit split for it. Against the 7 + 22 + 24
   list as written, `931g` resolves to 931 **grams** — the silent mistyping the document warns
   about, guaranteed rather than risked.
3. **Occurrence identity for the stated-unit record.** A per-occurrence record must name WHICH
   quantity in the term it is the surface of. Unambiguous for a single-quantity claim; not for the
   ten WRN sentences carrying two or three units.

## Blast radius: let the compiler count it

The D94 plan estimated 19 files by counting occurrences of a string. The real number was **six
exhaustive match sites**, and the estimate was wrong by 3×. So this plan does not estimate.

What is known: `PrimitiveType` is referenced by exactly two crates, `kernel` and
`eigenius-lean` (plus the latter's test). `Exp::` appears in 74 files, but that is an upper bound
containing constructions and non-exhaustive matches, not a work estimate.

D94's precedent for one new literal variant: `Exp::LitRat` flagged 4 sites, `Val::LitRat` 2, and
`PrimitiveType` 2 — one of those in `eigenius-lean`, which `-p eigenius-kernel` never compiles.
**Run `cargo build --workspace` to enumerate; it is one build and it is exact.**

## Slice 1 — `Unit` in the type theory, no chain change

A new `Exp` variant carrying a `units::Unit`, a matching `Val` variant, and `PrimitiveType::Unit`.

D93 records the cost this incurs: `eigentt:Term`'s literals are `LitInt/LitString/LitFloat/LitBool`
with **no `LitUnit`** — "a cost a primitive `Unit` pays and an inductive one would not".

Two things the compiler will NOT flag, both of which bit D94 silently:

- **`ground_values_equal`** (`nbe/eval/mod.rs`) has explicit per-literal arms, so a new one falls
  through to `false` and two identical units compare unequal. Canonical form makes the arm
  structural, exactly as for `Rational`.
- **`check_infer`** has explicit per-literal arms and an `Err(CannotInfer)` catch-all, so a new
  literal is untypeable until its arm exists.

Neither is a compile error. Add both arms and a test for each before moving on.

## Slice 2 — `Magnitude` → `Value::Embedded`

Blocked on decision 1. `Value::Embedded` is shape-anchored, so it survives the parser and CBOR
boundaries that D84 §5's tests rule a schema-discriminated variant out of.

## Slice 3 — chain surface AND SI content, then ONE reseed

The seam is wider than D94's, because the SI content is also bootstrap. Anything compiled in via
`include_str!` in `kernel/src/bootstrap/mod.rs` — 20 ontologies today — is part of the manifest, so
a units layer moves it just as `core-ontology.json` does. Both must land together.

- `core:unit` as a `DataType` in `core-ontology.json`, **and its entry in `core:data_type`'s
  `allows_only` set** — that set is closed, and omitting it fails the bootstrap outright (D94 hit
  exactly this).
- `Quantity : Unit -> Set` as a chain inductive, value-indexed by a unit.
- A new `eigentt:Term` constructor carrying a unit value, with its D47 mirror arms.
- The Eigon-JSON form, and the ESL printer and compiler arms.
- `ontologies/units/units.esl` as a 21st `BootstrapOntology`: 7 base, 22 named derived, 24 prefixes,
  the SI-accepted non-SI list (min, h, d, ha, L, t, Da, eV, au), and conversion factors as exact
  `core:rational` values. Under `urn:eigenius:units`, NOT `urn:eigenius:measurements` — that
  namespace is D52's statistics institution, and SI is not statistics.
- °C as a documented extension carrying the point assumption.

**Then one reseed.** `MALLOC_ARENA_MAX=1`, no stale container holding memory, and no wall-clock
timeout — the three things that made the 2026-09-23 run succeed after two failures. Update
`EXPECTED` in `kernel/tests/bootstrap_manifest_pinned.rs` in the SAME commit as the ontology edit,
as that test requires, then `scripts/build-alignment-snapshot.sh`.

## Slice 4 — the stated-unit record

Blocked on decision 3. At the encoding level, hung off `enc:DiscourseUnit`, which already carries
`enc:prose` and character offsets. NOT in the term: a datum every semantic operation must ignore is
provenance, not meaning.

## Deferred, with reasons

- **Implicit Π.** D93 calls it a general kernel change where "the blast radius is the whole type
  theory's binder" — every Π type gains the affordance, and it moves `eigentt:Term`'s `Pi`
  constructor and the D47 codec with it. It buys `mean {u} [x,y,z]`, which is ergonomics: explicit
  Π always works, so this is not a prerequisite for `Quantity : Unit -> Set`.
- **The Lean `Rat.mk'` literal proofs** (D94). The type maps; the literal refuses, pending
  `Decidable.decide` witnesses constructed in nanoda.
- **All of D95** — the parser work is consequent, not concurrent.

## What a completed D93 looks like

`37 °C` parses to a term the kernel type-checks as `Quantity(K)` with an exact magnitude, round-trips
through Eigon-JSON and ESL, and carries its authored surface on the encoding record rather than in
the term. Today none of that is expressible: `units.rs` is an island with 13 passing tests.
