# D93 implementation plan — units of measure

Ordered slices for the remainder of D93. [`kernel/src/units.rs`](../../kernel/src/units.rs) already
carries the algebra — `Unit`, `Exponent`, `Magnitude`, `BaseDimension`, `Kind`, `Constant`, and 13
tests including D93's three-row kind table. **Nothing outside that file references it.** Everything
below is the work of connecting it.

D94 is done and landed, so exact rationals are available for conversion factors.

## Three decisions block coding

None can be worked around, and each changes what gets written.

1. ~~**The embedded shape for `Unit` and `Magnitude`.**~~ **DECIDED — canonical string, not
   `Value::Embedded`.** See "The carrier for `Unit` and `Magnitude`" below.
2. ~~**`931g`.**~~ **DECIDED — refuse.** v1 does not split a `g`-suffixed numeral; `931g` stays
   unparsed rather than becoming 931 grams. Admitting standard gravity would open "units science
   uses that the SI does not accept", and `rpm` in the same corpus sentence queues up behind it, so
   the category gets opened deliberately with a criterion rather than one symbol at a time. D93
   records the full reasoning.
3. ~~**Occurrence identity for the stated-unit record.**~~ **DROPPED — there is no record.** Its one
   addition over `enc:prose` was making "which claims reported a dose in mg/kg" a query, and that is
   not a query this system needs. See D93, "Decided: no per-occurrence record".

**All three are closed.** Slices 1-3 are built; slice 4 is dropped.

## The carrier for `Unit` and `Magnitude` — decided

**A canonical string in `Value::String`, with EigenQL surface constructs for querying. Not
`Value::Embedded`.**

An earlier draft of this plan said `Value::Embedded`, on the grounds that a unit is *composite*.
That reason does not survive: a rational is composite too — numerator and denominator — and D94 put
it in `Value::String`. Composite-ness only rules out a SCALAR carrier like `Value::Float`. A string
is not a scalar carrier, it is a serialisation, and a CANONICAL composite serialises to one exactly
as `1/20` does.

What actually separates the two is that `Embedded` is a resource and a string is text. Three
consequences, and they run one way:

- **`core:mentions` edges.** `term_mentions.rs:93-99` is explicit: `Value::String(_) => {}` — "a bare
  string is not a reference" — while `Value::Embedded(r)` inserts every class in `r.is_a()`. So an
  embedded unit adds an edge per quantity occurrence, and D93 already names the cost: "every claim
  carrying a quantity acquires a dependency on [the units layer], and the justification
  well-foundedness check — transitive closure over `core:mentions` — grows accordingly." A string
  adds none.
- **Term size**, which D93 lists as a cost in its own right. `s^-1·m^2` is what `Unit`'s `Display`
  already emits.
- **Consistency** with the rational carrier, so one rule covers both.

**The one argument for `Embedded` was queryability** — dimension exponents are something you would
plausibly filter on, unlike a rational's numerator. That is answered better by dedicated EigenQL
constructs than by raw embedded fields, and the pattern is already in the system: `DATE`
(`query/functions.rs:33-47`) takes a canonical STRING, validates its format, and returns it. A date
is composite — year, month, day — carried as text and given meaning by a function.

So `UNIT(u)` and `DIMENSION(u)` join the six functions `call_function` already dispatches. That is
a better interface than filtering on a field named `dimension_2`, and it keeps the carrier
consistent with D94's. `COMMENSURABLE(a, b)` is not added: it is `DIMENSION(a) = DIMENSION(b)`.

**A magnitude has no carrier of its own.** It reaches a term as the two arguments of
`units:mk_quantity(coefficient : core:rational, pi : core:integer)` — see D93, "How a magnitude reaches a
term". Slice 2 first gave `Magnitude` a canonical string, serde and a `MAGNITUDE()` query function;
all three were removed once that was decided, because nothing on the chain stores one.

**Consequent on this:** canonical form must be established at construction, as `Rational`'s is, so
string equality is value equality and `conv` does no arithmetic. `units::Unit` already guarantees it.

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

## Slice 2 — the `Unit` string carrier, and the EigenQL constructs

Done (`24c09a5`). The canonical string form for `Unit` (parse and print, refusing non-canonical
input, as `Rational` does), serde over it, and `UNIT` and `DIMENSION` in `call_function`.

## Slice 3 — chain surface AND SI content, then ONE reseed

**Done.** Built in the commit adding `ontologies/units/units.esl`; reseeded 2026-09-24. As built,
the units layer loads after `prov` rather than directly after `core` (D93, "Load order"), and its
vocabulary adds the gram and the three plane angles to the lists below (D93, "As built").

The reseed loaded 9,440,702 resources across 32 layers with the Wiktionary countability list
provisioned — 43,474 WordNet and 1,543,129 UMLS additive mass entries — into
`db-snapshot/wordnet-umls-2026-09-24`; the aligned snapshot `wordnet-umls-aligned-2026-09-24` carries
40,375 alignment resources. Earlier snapshots on this machine were built without the countability
list, which is the likely cause of their 35,376 alignment resources against 40,357 elsewhere.

The seam is wider than D94's, because the SI content is also bootstrap. Anything compiled in via
`include_str!` in `kernel/src/bootstrap/mod.rs` — 20 ontologies today — is part of the manifest, so
a units layer moves it just as `core-ontology.json` does. Both must land together.

- `core:unit` as a `DataType` in `core-ontology.json`, **and its entry in `core:data_type`'s
  `allows_only` set** — that set is closed, and omitting it fails the bootstrap outright (D94 hit
  exactly this).
- `units:Quantity (u : core:unit) : Set` as a chain inductive with `u` a uniform parameter, as
  `core:Asserts` has `iri : core:string`. One constructor,
  `mk_quantity(coefficient : core:rational, pi : core:integer)`.
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

## Slice 4 — dropped

The stated-unit record was designed through several rounds and then dropped with the query it
served; see decision 3. What the author wrote stays in `enc:prose`, reachable from every claim
through `enc:from_unit`.

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
