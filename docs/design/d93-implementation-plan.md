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

So `UNIT(u)` joins the six functions `call_function` already dispatches. That is a better interface
than filtering on a field named `dimension_2`, and it keeps the carrier consistent with D94's.
`DIMENSION(u)`, which dropped kinds, was added and then removed with the kind vector (slice 5a);
commensurability is equality of canonical units, so `COMMENSURABLE` is not added either.

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
input, as `Rational` does), serde over it, and `UNIT` in `call_function` (`DIMENSION` was removed
in slice 5a).

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

## Slices 5 and 6 — found missing after slice 3

A check of D93's body against the code after slice 3 found two v1 obligations this plan never
listed. Neither is in D93's scope paragraph; both are in its argument. Designing the first found a
third problem, which comes first.

## Slice 5a — kinds out of the unit

D93's canonical form dropped every kind once the dimension vector was non-zero. That is not
associative — `(m·rad)·m⁻¹` gave `1` and `rad·(m·m⁻¹)` gave `rad` — and no group admits it, since
`m·rad = m` cancels to `rad = 1`. Open normalisation presupposes the group laws, so this blocked
slice 5. D93, "Kinds are metadata, not algebra", records the decision: `units::Unit` is the exponent
vector over the seven base dimensions alone, and kinds are metadata on the units layer (`units:kind`,
with `units:plane_angle` and `units:solid_angle`).

Removed with the kind vector: `Kind`, `Unit::kind`, `kind_exponent`, `dimension_only`, the `angle`
symbol in canonical strings, and the `DIMENSION()` query function, which only ever dropped kinds and
is now the identity. The group laws — associativity, commutativity, inverse, identity — are pinned as
tests. `core:unit`'s description and five `units.esl` entries change, so `core` and `units` move.

## Slice 5 — open unit expressions: `Quantity (u * v⁻¹)`

**What D93 requires.** "Normalisation is sufficient because Kennedy's normal form covers OPEN terms —
unit *variables* appear in it alongside base units. So a computed result type `Quantity (u * v⁻¹)`,
with `u` and `v` still variables, has a canonical form, and equality of two such is syntactic."
Today no function can return a ratio's unit.

**D5.1 — operators are declared constants, nanoda's pattern.** `units:mul : core:unit -> core:unit ->
core:unit` and `units:pow : core:unit -> core:rational -> core:unit`, axioms in `units.esl`. nanoda
extends Lean's kernel the same way: `Nat.add` and its siblings are ordinary constants recognised by
name (`tc.rs:395-458`), and the only new term nodes are the literals — here `LitUnit`, already built.
So no new `Exp` variant, `eigentt:Term` constructor, codec or ESL arm. The reduction hooks where an
axiom's application spine is built, `Val::app_impl`'s neutral arm (`nbe/val.rs`), which is where
nanoda's `try_reduce_nat` sits relative to its `whnf`.

**D5.2 — beyond nanoda: open products normalise.** nanoda reduces only closed literals —
`try_reduce_nat` returns early on free variables — so `x + y ≡ y + x` is not decided in Lean's
kernel. D93 requires the opposite for units, and commutativity is wanted wherever it applies. So a
product is normalised to: the closed part (a `Unit`) and ATOMS, each with a rational exponent,
merged when their read-back terms are equal and ordered by a deterministic key over that read-back.
It reads back as one canonical spine — the closed part as a `LitUnit`, then each atom in key order,
raised by `units:pow` when its exponent is not 1, right-nested under `units:mul` — so `eq_nf`
(`nbe/check/conv.rs:42`), which compares read-back terms structurally, decides equality with no new
comparison logic. With no atoms left the value IS a `LitUnit`. The rebuilt spine is constructed
directly, not through `app_impl`, and re-normalising it gives it back: normalisation is idempotent.

**D5.3 — an atom is any unit-valued neutral.** Not only a variable: an axiom application such as
`units:of(x)` commutes too. `Neut` has no ordering, so atoms are keyed by their read-back term. Two
atoms MERGE only when their read-back terms are equal — exactly `eq_nf`'s notion, so merging never
equates different units. The ordering key is the read-back term's rendering; the worst a key
collision can do is leave two equal products unrecognised, never make two different ones equal. The
read-back for keys runs at a level above every free variable, so a binder inside an atom cannot
take a free variable's name.

**D5.4 — `units:pow`'s exponent.** Reduces only when the exponent is a literal whose numerator and
denominator fit `Exponent`'s 16 bits; a larger literal is refused, not wrapped. A non-literal
exponent leaves the application stuck, and a stuck `pow` is an ordinary atom of any product it
enters.

**D5.5 — surface: function style.** `units:mul(u, units:pow(v, r"-1"))`. Infix `*`, `/`, `^` would be
ESL sugar desugaring to these calls, never kernel syntax; not built now.

**Verify before relying on it:** that `mean m [x]`'s result type, `Quantity(u)[u := m]`, evaluates
to `LitUnit(m)` — that instantiating a closure re-normalises — and that normalising a canonical
spine returns it unchanged.

`units.esl` gains the two axioms, so `units` moves again; one reseed covers 5a and 5.

**Built.** `nbe/unit_ext.rs`, hooked in `Val::app_impl`. On the commit path: a unit-generic
`ratio` applied to `m` and `s` has result type `Quantity(s^-1·m)`, and swapped it is refused as
`s·m^-1`; over unit variables `mul(v, u)` is accepted where `mul(u, v)` is expected, and `mul(u, u)`
is refused.

**Found while building: axioms could not mention axioms.** `Layer::axiom_env` is a `OnceLock` whose
initialiser type-checks every axiom's statement, and checking one that mentions another asked
`axiom_env` for that type — re-entering its own initialisation and deadlocking, both threads in
`futex_wait`. No axiom's statement had mentioned another until `ratio : … -> Quantity(units:mul(…))`.
It is independent of units: `axiom x : p("a") -> Prop` hung the same way. Admission is now on
demand inside the construction (`program::axiom_env::axiom_type`), and a statement that needs its
own axiom, directly or through others, is refused by name. `kernel/tests/
axiom_statements_mention_axioms.rs` runs each case under a watchdog, so a regression fails rather
than hangs.

**And the cache discarded every axiom on one failure.** It held
`build_axiom_env(..).unwrap_or_default()`: its doc said a malformed axiom would be dropped, and the
code returned an EMPTY environment, so one bad axiom made every axiom reference on the chain fail as
"not registered", with the reason gone. The cached build now keeps what admits and records why the
rest did not.

## Slice 6 — base conversion: authored units into base units

**What D93 requires.** "An author writing `5 g`, or an API caller passing grams, goes through a helper
that converts to base units … These live in the ESL elaborator and the Rust API." Nothing reads
`units:factor` today. No manifest change, so no reseed.

**D6.1 — where.** `units::convert` in the kernel crate, outside the checker, as D93 places it beside
D86's literal normalisation. The vocabulary is read from the chain's units layer
(`Vocabulary::from_layer`), not duplicated in Rust.

**D6.2 — the stated form.** A strict grammar shared by the ESL surface and the Rust API: factors
separated by `·` or a space, each a symbol with an optional prefix and an optional `^` exponent, and
at most one `/` with UCUM's meaning (every factor after it inverted). No `µ` (U+00B5), no superscript
digits, no `per`: D95's unit sub-parser normalises prose variants into this form and calls the same
converter.

**D6.3 — symbol resolution.** An exact symbol wins, then the LONGEST prefix plus a prefixable unit,
else refuse: `min` is the minute, `cd` the candela, `Pa` the pascal, `dam` deca-metre, `ms`
millisecond, `kDa` kilodalton, and `pH` — correctly, in ESL — picohenry.

**D6.4 — the affine rule.** The °C offset applies only when the stated unit is exactly `°C¹`, the
point reading D93 assumes. Inside a compound (`°C/min`) the reading is a difference and °C converts
as K with no offset.

**D6.5 — exactness.** A factor raised to a non-integer exponent must stay rational and give an integer
power of π, else refuse: `km^(1/2)` needs √1000. Results are bounded by D94's 4096 bits.

**D6.6 — kinds come back beside the unit.** Conversion factors the stated unit into its group element
and the kinds of its factors, numerator and denominator separately: `rad/s` is `s^-1` with a plane
angle in the numerator. The quantity term takes only the group element; the kinds are returned to
the caller as metadata (D93, "Kinds are metadata, not algebra").

**D6.7 — lowering.** `Magnitude` → `(coefficient, pi)` is total while `Constant` has one variant;
raising a stored `pi` refuses a value outside `i16`. The output term is
`(units:mk_quantity(r"c", pi) : units:Quantity(u"dim"))` — annotated, because `u` is a parameter.

**D6.8 — the ESL surface: function style.** `units:quantity(5r, "mg/kg")`, elaborated in the compiler,
which has the chain; it produces the annotated term above.

**Verify:** a round trip for every entry in `units.esl`, and that `37 °C`, `5 mg/kg`, `50 kDa` and
`37 °` produce the magnitudes D93 states.

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
