# D93 — Units of measure

**Written** `2026-09-19`, against `main` at `1efd6bb`. Supersedes the placeholder at
`docs/notes/d52-d62-numbers-and-measurements.md` §5 piece 3, which identified this work and deferred
it ("the biggest and most genuinely design-first piece… deserves its own deliberation and prior-art
grounding"). **First draft.** Depends on **D94** (`core:bigint` / `core:rational`) for exact
conversion factors and prefixes.

## The gap

The system holds no notion of a unit of measure. Not a weak one — none. `grep` over `ontologies/`
finds no unit vocabulary at all; the one hit for `enc:unit` is a *discourse* unit, a span of text.
D52 punts units to `core:string`. The `measurements:` namespace carries quantities (`alpha`,
`computed_p_value`, `effect_size`) and D86's order relations, but nothing that says what a number
counts.

So a measured quantity is a bare `core:float` whose unit, if recorded at all, is a string nothing
checks. `24` and `24` are the same term whether they are hours or milligrams.

For a platform whose premise is checkable scientific claims, that is a hole in the foundation rather
than a missing convenience. Dimensional consistency is the first check a scientist applies to an
equation; the system cannot apply it.

## The decision

**A unit is a kernel primitive, and unit errors are type errors.**

Concretely: `Unit` as an `Exp::EigonPrimitive`, and quantities typed `Quantity : Unit -> Set`, so
that a unit mismatch fails to type-check rather than failing a validation rule.

This is the first deliberate expansion of the native type checker's trusted surface (the paper's §8
inventory: native checker, hosted external checkers, comorphisms, the attribution constant
specification). The expansion is small and stated below.

## The theory: units are an Abelian group

Andrew Kennedy, *Types for Units-of-Measure: Theory and Practice* (CEFP'09 / Types At Work 2009;
`references/papers/`), is the design's basis. Units-of-measure form an **Abelian group** over
base units — Kennedy's words; the further claim that it is *free* (over ℤⁿ) is this document's, and
holds only for the integer-exponent reading he assumes — associativity, commutativity, identity, inverses. Three properties make it
implementable rather than merely elegant:

- **Unique normal form** (§3.3). Every unit expression reduces to
  `α₁^x₁ * … * αₘ^xₘ * b₁^y₁ * … * bₙ^yₙ` with non-zero exponents, unit variables and base units in
  a fixed order. Deciding `u = v` is *normalise, then compare syntactically*. Kennedy's exponents
  are integers; ours are rational — see "Exponents are rational" below.
- **The theory is unitary** — it possesses most general unifiers. Kennedy notes this is rare (he
  names Boolean rings as "one other" — not as the only other).
- **Unification is decidable** by Gaussian elimination, reducing `u = v` to `u * v⁻¹ = 1`.

The semantics is worth stating because it is what makes unit-correctness a claim about the world
rather than a runtime guard. Kennedy grounds it in **invariance under change of unit system** —
"physical laws are independent of the units used" — and shows this is relational parametricity over
a scaling environment, yielding free theorems — Kennedy's example is that in a total language the
only function of type `∀α. float<1> → float<α>` is the constant zero, "because by invariance under
scaling we must have f(x) = k * f(x) for any k". Instrumenting the semantics to trap mismatches he calls "cheating… we have instrumented the
semantics and thereby changed it".

## Exponents are rational

**Decided.** The exponent on a base unit is a reduced fraction, not an integer. In practice the
denominator is 1 for nearly every unit; the cases where it is not are standard, not exotic:

| unit | exponent | field |
|---|---|---|
| `nV/√Hz`, `pA/√Hz` — amplitude spectral density | `s^(1/2)` | electronics; every amplifier datasheet |
| `MPa·√m` — fracture toughness `K_IC` | `m^(1/2)` | materials, ASTM E399 |
| `s·m^(−1/3)` — Manning's roughness coefficient | **cube root** | hydraulics |
| `Ω·s^(−1/2)` — Warburg coefficient | `s^(−1/2)` | electrochemical impedance |
| `g^(1/2)·cm^(3/2)·s^(−1)` — statcoulomb | half-integers throughout | CGS-Gaussian electromagnetism |

Manning's `n` is why the requirement is *rational* and not merely half-integer.

**Taking a square root does not force this** — and in v1 the question does not arise, because
`sqrt`-shaped signatures are out (see Scope). Kennedy types `sqrt : float<'u ^ 2> -> float<'u>`,
constraining the input to a perfect square so a root never produces a fractional exponent; typing an
*application* of that signature would need AG-unification, which v1 excludes.
Standard deviation and RMS carry the quantity's own unit, and variance is squared. What forces
rational exponents is a *stated* unit that is irreducibly fractional, which is what the table lists.

**Why decide now rather than later.** `Unit` is a kernel primitive whose terms are stored on chain.
Changing the exponent representation afterwards moves the bootstrap manifest, which makes every
persisted store unresumable and forces a full reseed. Deciding now costs the element type of a
vector and turns exponent addition into rational addition — total, trivial, and exercised almost
never.

**What this changes relative to Kennedy, honestly.** With integer exponents, units form a *free*
Abelian group (ℤⁿ). With rational exponents they form a ℚ-vector space (ℚⁿ), which is divisible and
therefore not free. Three consequences:

- **Normalisation is unchanged** — a vector of reduced fractions still has a unique canonical form,
  and sorting and dropping zero exponents works identically.
- **The semantics survives, and arguably fits better.** Kennedy's scaling environment ψ maps unit
  variables to **positive** scale factors, and positivity is exactly what makes `ψ(u)^q` well
  defined for rational `q`. The parametricity argument does not depend on integrality.
- **Unification would differ.** Kennedy's Gaussian elimination solves linear *Diophantine*
  equations; over ℚ the same problem is ordinary linear algebra. Since v1 defers unification this
  costs nothing now, but re-establishing the most-general-unifier property over ℚ is part of
  whatever later brings it into scope, and should not be assumed from the paper.

## The algebra, stated once: exponent vectors and a coefficient

Earlier drafts conflated three structures. Separating them is what keeps symbolic algebra out of the
kernel.

| | structure | kernel operations |
|---|---|---|
| unit exponents | ℚ⁷ — a vector space over the seven base dimensions | add, subtract, scalar-multiply, compare |
| magnitude | ℚ × ℤ^C — a rational coefficient and integer powers of declared constants | **compare only** |

Both are exponent vectors over a symbol set; a magnitude is that plus a rational coefficient. No
expressions anywhere — which is why the kernel needs no symbolic algebra. (A third row, kind
exponents, was here and is withdrawn: see "Kinds are metadata, not algebra".)

**A magnitude is a canonical datum, not a formula.** `q × Π cᵢ^{eᵢ}`, canonicalised by reducing `q`,
sorting the constants and dropping zero exponents; zero is `(0, [])`. So `37π/180` and `π·37/180`
are both `(37/180, [π ↦ 1])` and compare syntactically. There is nothing to reorder, which is the
point: making `37 × π` and `π × 37` compare equal *as expressions* would need AC-normalisation, and
that is symbolic algebra in the trusted surface.

**It is a multiplicative group and deliberately not a ring.** `37π/180 + 1/2` escapes the form
entirely — magnitudes are not closed under addition. They do not need to be: the kernel's only
magnitude operation is equality. Unit normalisation adds *exponents*, not magnitudes; conversion
multiplies, and happens at ingest outside the TCB.

Where addition would be needed — anything that computes with quantities rather than checking them —
the closure is `ℚ[π, π⁻¹]`, and with division `ℚ(π)`. Because π is transcendental that field is
isomorphic to `ℚ(x)`: π is an indeterminate, equality reduces to comparing rational functions in
lowest terms, and no transcendence question arises. It costs **polynomial** gcd rather than integer
gcd, and it belongs to an institution or a program. Never the kernel.

### The constant set is declared, and v1 declares `{π}`

**With one constant the canonical form is provably a normal form.** `q₁·π^{k₁} = q₂·π^{k₂}` would
require `q₁/q₂ = π^{k₂−k₁}`, and since π is transcendental (Lindemann, 1882) `π^n` is irrational for
every `n ≠ 0`. So the exponents and coefficients must match, and syntactic equality of canonical
forms **decides** value equality. A theorem, not an assumption.

**With two or more it rests on an open problem.** The requirement is that the constants be
multiplicatively independent modulo ℚ — no product of integer powers equal to a rational except
trivially. Whether π and ln 2 satisfy that is **unknown**; each is individually transcendental (ln 2
because otherwise `2 = e^{ln 2}` would be transcendental, by Lindemann–Weierstrass), but their joint
independence would follow from Schanuel's conjecture and is unproved.

**The failure mode is incompleteness, not unsoundness.** A hidden relation would make two magnitudes
denoting one real compare *unequal* — the kernel refuses an equality that holds, never asserts a
false one. That is the D86 posture, and deciding real equality is undecidable in general regardless.

**And the boundary coincides with one this document already drew.** Every constant beyond π is
needed by units v1 already excludes:

| constant | needed by | v1 |
|---|---|---|
| π | angles, parsec, atomic units | **in**, provably decidable |
| ln 10, ln 2 | neper, bel, decibel | already out — logarithmic units |
| e | nothing in scope | — |

So `{π}` is not a convenience. It is the exact closure of the unit set v1 admits, and admitting a
second constant means admitting logarithmic units, which carry their own problems. The set is a
**declared** part of the primitive, and the point at which a second constant is added is the point
at which the normal-form guarantee changes from proved to assumed — which must be written down then,
not discovered later.

## Normalisation only; parameterisation instead of unification

**The kernel primitive implements normalisation. It does not implement unification.**

Kennedy needs Gaussian elimination because F# *infers* units under Hindley–Milner. EigenTT is
dependently typed, so a unit can be bound rather than solved:

```
mean : (u : Unit) -> List (Quantity u) -> Quantity u
```

An ordinary Π. Checking `mean m [x,y,z]` instantiates `u := m`; nothing is solved for. This is
expressible with no new binder machinery: `Exp::EigonPrimitive(_)` infers to `Sort 1`
(`kernel/src/nbe/check/mod.rs:1320`, "ground types in `Set`"), so `Unit` is a valid Π domain.

**Normalisation is sufficient because Kennedy's normal form covers OPEN terms** — unit *variables*
appear in it alongside base units. So a computed result type `Quantity (u * v⁻¹)`, with `u` and `v`
still variables, has a canonical form, and equality of two such is syntactic.

The division of labour:

| | decides | needed for |
|---|---|---|
| normalisation | equality of unit expressions, open or closed | checking, parameterised functions |
| unification | solving for a variable inside a product | inference of unit variables |

Against D52's actual computations this suffices: `mean_of` and `mean_diff_of` are
same-unit-in/same-unit-out; p-values and `alpha` are dimensionless (`Quantity 1`, no variable);
effect sizes are standardised-dimensionless or in the measurement unit; ratios produce a computed
`u * v⁻¹` that normalises.

**What would reopen unification** is a unit-generic *derived* quantity where a variable carries a
non-unit exponent or two variables interact (`α² * β = m³ * s`). Not the statistics institution.
Deferring costs nothing now. Whether the unifier *composes* later is open, not safe: over ℚ the
group is divisible rather than free, and Gaussian elimination over linear Diophantine equations is a
different algorithm from linear algebra over ℚ. This document says exactly that two sections below,
and an earlier draft asserted the opposite here.

**Implicit unit arguments were in — deferred as of 2026-09-25, eigenius#261.** `mean {u} [x,y,z]` with `u`
inferred is implicit-argument solving, and it stays first-order — `Quantity ?u ≡ Quantity m` is
syntactic — precisely when **every implicit unit variable appears alone as the index of at least one
explicit argument's type**. A signature violating that, such as `c : {u} -> Quantity (u * u)`,
requires `?u · ?u ≡ m · m`, which is AG-unification and stays out of v1. Those signatures are
rejected rather than half-supported; the explicit Π is always available.

**What it costs is larger than units.** The existing `implicit(…)` is `InductiveCtorDecl::implicit`
— a per-constructor declaration on inductive types, whose only user is `justification:Grounds`.
`Exp::Pi` carries no implicitness at all, so implicit function arguments do not exist in EigenTT
today. Admitting them is a **general** kernel change — every Π type gains the affordance, not only
unit-indexed ones — and it moves `eigentt:Term`'s `Pi` constructor and the D47 codec with it. Units
motivate it; they are not the only beneficiary, and the blast radius is the whole type theory's
binder.

**Superseded: it is an elaboration feature, not a kernel change** (eigenius#261). nanoda's `Pi`
carries a `binder_style` that its checker never consults — `def_eq_binder_aux`
(`references/nanoda_lib/src/tc.rs:873`) compares binder types and bodies only — because in Lean the
ELABORATOR inserts implicit arguments and the kernel checks fully explicit terms. Eigenius's own
`InductiveCtorDecl::implicit` already works that way: only `term_mentions` reads it. So the binder,
the codec and stored terms need not change; the work is making the ESL compiler infer argument types
at application sites where implicit binders occur. And solving a unit unknown is simpler than the
AG-unification exclusion above assumes: since "Kinds are metadata, not algebra", units form a vector
space over ℚ, so one unknown `?u^k · A = B` has the unique solution `?u = (B · A⁻¹)^(1/k)` —
`{u} -> Quantity(u * u)` against `Quantity(m²)` gives `u = m` — and several are a linear system.
The exclusion comes from Kennedy's integer, Diophantine setting. Not yet verified beyond the
argument.

## What is trusted, and how little

**Only the seven base units.** F# puts the algebra in the language and the SI content in a library —
its PowerPack "declares all of the SI base and derived units". Kennedy is explicit that derived units
are definitional: `[<Measure>] type N = kg m/s^2`, and "`N` and `kg m/s^2` mean exactly the same
thing".

So the 22 named derived units and the 24 prefixes are **abbreviations that normalise away**, not
additional primitives. The trusted surface is:

- seven base-unit symbols (second, metre, kilogram, ampere, kelvin, mole, candela),
- a rational exponent vector over them,
- a declared constant set — `{π}` in v1 — and integer exponents over it,
- a canonicalisation: sort, reduce the fractions, drop zero exponents.

That is the whole TCB addition. The SI *content* lives in chain ontology where it is authored,
reviewed and replaceable without touching the checker.

Quantity kinds are not in the trusted surface. An earlier version of this section put a kind
exponent vector there; it is withdrawn, and kinds are metadata on the units layer's vocabulary
(see "Kinds are metadata, not algebra").

## Dimension is not the whole of a unit

`rad`, `sr`, `°`, `%` and `ppm` are all dimensionally 1. A dimension vector alone therefore unifies
them, and unifying a plane angle with a solid angle, or a ratio with an angle, is wrong.

The resolution is that these are not dimensional quantities at all. They are **scaled counts and
ratios**, and they differ from each other in *kind*, not in dimension.

The SI's own language has moved in the direction this argues. The 8th edition (2006) called the
radian and steradian "special names for the number one"; the 9th dropped that, and the current
edition says of `rad = m/m` that "this representation is not intrinsic and may be misleading since
angle is not the same kind of quantity as other length ratios" — which is the kind/dimension
distinction in the standard's own words. Note also that **`sr = rad²` appears in no edition**: it is
derivable from the 2019 printing's Table 4, which the current edition removed.

| | SI status | content |
|---|---|---|
| `rad` | derived unit, dimensionless | m/m = 1 |
| `sr` | derived unit, dimensionless | m²/m² = 1 |
| `°` | non-SI, accepted for use with the SI | π/180 rad |
| `%` | not a unit — notation for a number | 0.01 |
| `ppm` | not a unit | 10⁻⁶ |

**Withdrawn — see "Kinds are metadata, not algebra" below.** *What follows, down to the QUDT
paragraph, was the decision and is kept as the record of what failed.* A unit carries a **kind
exponent vector** alongside its dimension vector — the same structure, over a set of quantity kinds
rather than base dimensions. So `rad` is `angle¹`, `sr` is `angle²`, and `sr = rad²` falls out
rather than being denied.

**And kind exponents are carried only when the dimension vector is zero, discarded otherwise.**
Without that rule a multiplicative kind vector re-breaks `s = rθ`: arc length is a length times a
radian, the dimension works (`m × 1 = m`) but the kind multiplies too, leaving a length that carries
`angle¹`. Which is precisely why an eighth base dimension was rejected below.

| | dimension | kind | result |
|---|---|---|---|
| `rad · rad` | 0 | `angle²` | `sr` — kept, dimension is zero |
| `m · rad` (arc length) | `L¹` | discarded | a plain length |
| `m / m` | 0 | none | dimensionless, and distinct from `rad` |

The rule is principled rather than a patch: the kind axis exists *because* dimension cannot separate
dimensionless quantities. Once a quantity has a dimension, dimension does the separating and the
kind has no work left.

**This was stronger than the QUDT precedent.** QUDT has `PlaneAngle` and `SolidAngle` as distinct
`qudt:QuantityKind`s sharing one dimension vector (`A0E0L0I0M0H0T0D1`) — a *classification*. It does
not say `SolidAngle = PlaneAngle²`. Giving kinds an algebra was a deliberate step past it — and it is
the step that failed.
`%` and `ppm` are not kinds but **scales** on the plain dimensionless unit, which is what they
actually are.

**Angle as an eighth base dimension was considered and rejected.** It distinguishes `rad` from `sr`
elegantly (`sr = rad²` falls out, though no edition states it) but breaks `s = rθ`: with angle dimensional, arc length would come
out as metre·angle rather than metre, and recovering it requires introducing a constant θ₀ = 1 rad
throughout. The SI keeps angle dimensionless for this reason, and it remains rejected.

## Kinds are metadata, not algebra — decided

**The kind vector could not work, and the reason is not a detail.** It made three commitments:

1. units form a group — Kennedy's, and the basis of this whole document;
2. `rad ≠ 1` — the reason the kind axis existed;
3. `m·rad = m` — the discard rule, so that `s = rθ` gives metres.

In a group, `m·rad = m` cancels to `rad = 1`. So no group holds all three. The implementation showed
it concretely: `(m·rad)·m⁻¹` gave `1` while `rad·(m·m⁻¹)` gave `rad`, so the product was not
associative and a unit's canonical form depended on how its expression happened to be grouped. It
went unnoticed while nothing multiplied units at the term level, and it blocked open normalisation
outright, since commuting and regrouping factors presupposes the group laws.

**Decided: a unit is the group element alone, and a kind is metadata.** `core:unit` is the exponent
vector over the seven base dimensions and nothing else — the free Abelian group, associative and
commutative, with `rad`, `sr`, `°` and `1` all its identity, as the SI has them, so `s = rθ` gives
metres. The units layer records a kind on the named units that need one — `units:kind` is
`units:plane_angle` on rad, °, ′ and ″, and `units:solid_angle` on sr — and a kind never enters a
unit's value or type equality.

This is QUDT's model, a classification over a shared dimension vector, which the withdrawn decision
set out to go past. The other consistent option — angle as an eighth base dimension, keeping
`rad ≠ 1` by giving up `m·rad = m` — stays rejected for the `s = rθ` cost recorded above.

**What is given up.** The type checker no longer refuses adding a plane angle to a solid angle, or a
ratio of lengths to an angle: all are `Quantity(1)`. That distinction now lives where metadata is
consulted — in the vocabulary, and in conversion, which returns a stated unit's kinds beside its
group element — not in type checking. `sr = rad²` is no longer derived; it is two recorded kinds.

## `mol` stays a base dimension

**Decided**, and against the grain of the section above — `mol` is a count too, scaled by N_A, and
the 2019 redefinition arguably strengthened that reading by making the mole a pure number of
entities.

It stays dimensional because treating it as a kind loses checks that matter in the domain:

- `kg` and `kg mol⁻¹` become the same type, so adding a mass to a molar mass would check.
- `mol m⁻³` (concentration) and `m⁻³` (number density) become the same type. They are the same
  physical thing up to N_A, which is exactly why the SI keeps them dimensionally distinct.

Chemistry is in scope for a research platform, and molar mass and concentration are where
dimensional errors actually occur there. Seven base dimensions catches more real errors than the
purist six.

## Where it lives: `urn:eigenius:units`

**Decided.** `units:Quantity`, `units:Unit` and the whole SI content sit in their own namespace and
their own bootstrap layer. The namespace is unused today.

Not under `urn:eigenius:measurements`: that namespace is the D52 *statistics* institution — it holds
`alpha`, `computed_p_value`, `effect_size` and D86's order relations. SI is not statistics. A
measured length has a unit whether or not anyone runs a test on it, and a system that made the unit
vocabulary a dependent of the statistics institution would have the layering backwards.

The layer holds:

- `units:Quantity : Unit -> Set` and the chain mirror of the `Unit` primitive,
- the seven base units, which are the only trusted symbols,
- the 22 named derived units and the 24 prefixes, as definitional abbreviations,
- the SI-accepted non-SI units (min, h, d, ha, L, t, Da, eV, au),
- conversion factors between commensurable units.

**As built (`ontologies/units/units.esl`), the vocabulary departs from that list in four ways:**

- **The gram is included.** It is not a base unit and is in none of the lists above, but the SI forms
  every mass multiple by prefixing the gram (`mg`, `μg`), never the kilogram. `units:gram` carries
  dimension `kg` and factor `1/1000`; `units:kilogram` is not prefixable.
- **The degree, arcminute and arcsecond are included.** They are in the SI Brochure's accepted table
  (Table 8) with min, h, d, ha, L, t, Da, eV and au, and the π-carrying magnitude exists to admit
  them. The logarithmic units in the same table (Np, B, dB) stay out.
- **Standard gravity is admitted, outside the SI's accepted list** (`units:standard_gravity`, `g_n`),
  under the criterion in "A vocabulary hazard the same evidence surfaced": exactly defined and
  attested in the corpus.
- **The degree Celsius is not prefixable.** A prefix on an offset unit is well-defined only for a
  difference, and v1 assumes the point reading.
- **Several named units share a unit value, as in the SI.** rad, sr, °, ′ and ″ are all `1`; Hz and
  Bq are both `s^-1`; Gy and Sv are both `s^-2·m^2`; lm is `cd`. Kinds, recorded as metadata
  (`units:kind`), distinguish the angles; v1 records only those two kinds. The prose keeps which
  unit the author wrote.

**Load order.** After `core`, which supplies the primitive types the magnitude rests on, and before
`statistics`, whose quantities carry units. **Also after `prov`**, which this document originally
missed: the layer is authored in ESL, and the ESL compiler stamps `prov:was_attributed_to` on every
declaration, so a slot directly after `core` fails the bootstrap with an unresolved property. It
loads immediately before `statistics`.

An earlier draft placed it before `formulas` and created a circularity: it also put conversion
factors — including the symbolic `1° = π/180 rad` — *inside* the units layer as `FormulaTerm`
values, and a layer holding `FormulaTerm` values cannot load before `formulas`. Retargeting to
`eigentt:Term` dissolves that: the units layer holds no `formulas:` values, so its position is
unconstrained relative to `formulas`.

Adding a bootstrap layer moves the manifest and obliges a reseed. That is the same cost B6 and D89
paid, and it is the reason the placement is settled here rather than discovered during
implementation.

## Why not the alternatives

**Units as an ordinary inductive, normalised in the object language.** `Exp` already has
`Data`/`Con`/`Case`, so a `Unit` type and `normalize : Unit -> Unit` are expressible today with zero
TCB growth — the discipline D86 chose for float literals (canonical form in the term, arithmetic out
of the checking loop). Rejected because unit equality would then be propositional, not definitional:
`Quantity (m * s⁻¹)` and `Quantity (s⁻¹ * m)` would be provably-equal types rather than the same
type, and every comparison would carry a normalisation redex through NbE. "Unit errors are type
errors" is the requirement, and this does not deliver it.

**Units via `NativeDecide` / an institution constraint.** The machinery exists —
`NativeDecide(Constraint::Institution(…))` reduces to `Refl` on a satisfied constraint
(`kernel/src/nbe/term.rs:135`). Rejected for the same reason, more sharply: unit agreement becomes a
checked side condition, which breaks the type-level equality that parameterised functions depend on.

**Units in `FormulaTerm`.** Earlier drafts of this document treated `formulas:FormulaTerm` as the
place units live. **That was a category error.** Propositions are `eigentt:Term` values, and a
quantity inside `lt(dose, threshold)` has to be expressible there. `FormulaTerm` is something else
by its own description — "the shared formula language across every numerical institution
(Symbolics, IntervalArithmetic, JuMP, DiffEq, Catalyst)" — an exchange format for handing formulas
to external solvers. The capability gap follows: `eigentt:Term` has 22 constructors including
`CtorApp` and `ConstRef`; `FormulaTerm` has six (`Var | LitFloat | OpRef | App | Lam | Pi`) and a
single float literal.

So `formulas:` keeps a role, but a later and smaller one: a **projection** so numerical institutions
can receive quantities. Out of v1. Putting units there would also leave the kernel unable to check them,
since the kernel checks `eigentt:Term`.

## Affine units are an extension, not part of v1

°C is the only affine unit in the SI — the other 21 named derived units are multiplicative and sit
inside the group unchanged. Fahrenheit and Rankine are non-SI and out of scope.

Affine units are **outside Kennedy's theory**: the free Abelian group is multiplicative, and the
paper addresses offsets nowhere. So °C needs its own construction.

**v1 normalises °C to K at ingest, under an explicit assumption.** The assumption is necessary
because °C is used for two different things, and the SI says so: a temperature *point*
(`heated to 85 °C` → 358.15 K) and a temperature *difference* (`rose by 5 °C` → 5 K, **not**
278.15 K, because the degree Celsius is equal in magnitude to the kelvin). Nothing in the unit
distinguishes them; the disambiguator is in the prose.

**Not "to" versus "by", which this document assumed and D95 refuted.** `at` carries both readings
(`incubated at 37 °C` is a point, `sampled at 5 °C intervals` a difference), and three constructions
carry the difference reading with no preposition at all (`the temperature rose 5 °C`, `a 5 °C
increase`, `5 °C warmer`). The measure phrase is neutral and its **consumer** supplies the reading —
prepositions by vector semantics, scalar-change verbs by the measure-of-change function, the
comparative morpheme by arithmetic. D95 records the evidence and the mechanism.

The general rule this fixes: when a consumer takes the **vector** reading of a measure phrase in an
affine unit, the result carries the associated vector unit. °C is the only affine derived unit in the
SI, so the rule has exactly one instance — °C difference lands in K.

So affine handling is partly a *grammar* concern, not purely a units one. v1:

- assumes the **point** reading for a bare °C, which dominates in methods prose (`85 °C`, `37 °C`),
- records the **difference** reading as a known gap until the grammar can disambiguate — the
  mechanism is now settled (D95, "The quantity is neutral"), the grammar work is not yet done,
- **normalises the value but never loses the authored unit** — storing 358.15 K where the source
  span reads "85 °C" is acceptable only because `enc:from_unit` keeps the surface recoverable. "What
  the author wrote" is a separate proposition from "what the quantity is", and this system says so
  elsewhere.

## Conversion factors are exact rationals, which is a dependency on D94

A conversion factor between commensurable units is an **exact rational**, held as chain data in the
units layer. Not a float: 1 lb = 0.45359237 kg and 1 inch = 0.0254 m are exact *by definition*, and
representing an exact definition approximately is the defect this specification exists to remove.

**This makes D93 depend on D94.** Exact rationals at SI scale do not fit 64 bits — eight of the 24
prefixes exceed `u64`, and 1 eV = 1.602176634 × 10⁻¹⁹ J needs a denominator of 10²⁸. D94 supplies
`core:bigint` and `core:rational`; D93 consumes them.

**Three kinds of factor, and only the first is a rational:**

- **Exactly rational** — most of them, many by definition since the 2019 SI: 1 h = 3600 s,
  1 au = 149597870700 m, 1 lb = 0.45359237 kg, every prefix as a power of ten.
- **Defined geometrically, giving a transcendental factor.** This is a *category*, not the single
  exception an earlier draft implied by naming only `1° = π/180 rad`:
  - the whole **angular family** — degree, arcminute, arcsecond, gradian, revolution, and
    square-degree to steradian through π²;
  - the **parsec**, which is a *length*: IAU 2015 defines it as exactly `648000/π` au, so
    `1 pc = 149597870700 × 648000/π` m. A dimensional unit with a transcendental factor and nothing
    angular about it once converted;
  - **atomic units**, which reach π through `ħ = h/2π` — exactly, since `h` is fixed post-2019.

  **The factor is not an expression.** Every member of this category has the form
  `rational × π^k` — `(37/180, π¹)` for the degree, `(149597870700 × 648000, π⁻¹)` for the parsec,
  `(1/32400, π²)` for the square degree — so it is a canonical datum in the same shape as a unit,
  not a formula in `eigentt:Term` or `formulas:`. See "The algebra, stated once" above. An earlier
  draft placed it in `eigentt:Term` as a `ConstRef` expression, which would have required
  AC-normalisation to compare.

  **Base-units-only sharpens this.** Since the chain admits no unit but the base ones, a degree
  quantity must be converted before it is stored, and its converted magnitude is irrational. So the
  magnitude carrier decides whether this category is expressible: strictly-rational puts every unit
  in it out of v1 by consequence rather than by choice.
- **Measured, carrying uncertainty** — the dalton is 1.660 539 068 92(52) × 10⁻²⁷ kg (CODATA 2022),
  a measurement rather than a definition. It is also the case in point: an earlier draft quoted the
  CODATA 2018 value, superseded in 2024, while arguing in this very paragraph that a measured factor
  must not masquerade as exact. A measured constant carries a **vintage** as well as an uncertainty,
  and the chain must record both.

  **Decided: `Da` stays in v1 as a documented exception.** Base-units-only removed the alternative —
  it can no longer keep its own base and defer the conversion — so `50 Da` becomes kilograms at
  ingest by multiplying through a measured constant. The exception is that this conversion is
  **not** exact and must not present as though it were. A reader can tell a magnitude derived
  through a measurement from one derived through a definition without a per-quantity record:
  `units:dalton` carries `units:factor_uncertainty` and `units:factor_source` (CODATA 2022), the
  units layer is bootstrap, and a store cannot be resumed under a different manifest — so the
  constant and its vintage are fixed for every quantity in a store, and the authored `Da` is in the
  prose. Every other SI-accepted unit in v1 converts through an exact factor.

  This is narrower than it sounds — `Da` is the only measured-factor unit in v1's list. It is also
  the one that will recur: any unit defined by a measured constant rather than a fixed one lands
  here, and the mechanism is what makes admitting the next one a decision rather than an accident. A conversion factor that is itself a measurement belongs to D52, not
  here, and must not masquerade as exact.

The second and third categories are small but they are not edge cases to be discovered later: angle
conversion is common, and any unit defined by a measured constant falls in the third. A design that
assumes every factor is a rational numeral is wrong about two of the three.

## A quantity carries both a stated and a normalised unit

The A/B fork above is a false one. A quantity can carry **both** — a normalised unit that the type
is indexed by, and a stated unit recording what the author wrote.

That resolves the two purposes without trading them off:

- **The type is indexed by the normalised unit.** Dimensional errors are type errors, and `24 h`
  and `86400 s` have the *same* type and unify — which is what lets two papers' claims meet.
- **The stated unit is data, not type.** It records `mg/kg`, and a stipulation about a unit is an
  ordinary proposition over that datum rather than something the type system must encode.

Coercion disappears as a question: nothing converts at check time because everything already
normalised at ingest, and the authored form was not discarded to achieve it.

**Where the stated unit sits** was reopened once and settled the other way; the candidates and the
argument that moved it are below.

**In the term, participating in equality.** Rejected: `24 h` and `86400 s` would then be distinct
terms, and claims from different papers would not unify — losing the property that motivated
normalising at all.

**In the term, erased before equality.** A new node is needed, and `Ann` is the wrong precedent for
it. `Ann` is erased by **`eval`** — `eval(Ann(e,_)) = eval(e)`, "so NbE normal forms never contain
`Ann`" (`kernel/src/nbe/term.rs:116`) — and that is the *counter*-evidence: a node erased by `eval`
does not survive anything that stores a normalised term, which is most of the round-trip surface
(`eigentt:Term.Checked`, a quoted witness, anything through quote∘eval).

What the stated unit needs is the opposite pairing: **preserved by `eval` and `readback`, ignored by
`conv`.** That is a different mechanism from `Ann`, not a variation on it, and its behaviour must be
specified separately in each of the three rather than by analogy. `Ann`'s second slot being a type
rather than a data slot is the lesser problem.

Its cost is queryability. A datum inside a term is not reachable from EigenQL, which cannot decode
terms — the same limitation the justification vocabulary records for warrant projections. "Which
claims reported a dose in mg/kg" would not be a query.

**Beside the term, as a property on the resource.** Queryable immediately, since resource properties
are what EigenQL is good at. But one property cannot disambiguate **several quantities in one
proposition** — `lt(dose, threshold)` with the dose stated in mg/kg and the threshold in mg/dL has
two stated units and one slot.

**Decided: at the encoding level, on the source-span record — not in the term.**

An earlier draft decided the opposite, putting the stated unit in the term. Its argument against a
resource-level record was that one property cannot disambiguate **several quantities in one
proposition**: of 240 WRN methods sentences, nine carry two distinct units and one carries three.

> "The flask was placed on a rotator and incubated at 37 °C for 1 h."
>
> "…the plates were spun at 931g for 2 h at 30 °C."

That argument assumed **one property on the claim resource**. The pipeline already carries something
finer. `enc:EncodedClaim` *requires* `enc:from_unit`, the link back to the source span
(`ontologies/encoding/encoding.esl:369, 409`), and the span it reaches — `enc:DiscourseUnit` — holds
`enc:prose`, the verbatim source text, with `enc:span_start` and `enc:span_end` (`:33-42`). A stated
unit keyed to a **sub-span** is not one property: nine sentences with two units are nine records
carrying two, and the sentence with three carries three. The data is exactly representable.

**The tell that it never belonged in the term.** This document's own requirement for the term node
was *preserved by `eval` and `readback`, ignored by `conv`*. A datum every semantic operation must
ignore is not semantic content — it is provenance about the text, and the system has a provenance
layer that is already mandatory on every encoded claim.

**What this avoids**, all of which the in-term decision had accepted as scope:

- **Queryability.** Resource properties are what EigenQL is good at, so "which claims reported a
  dose in mg/kg" becomes a query now rather than after terms become decodable.
- **Reference edges.** No unit IRIs inside terms, so `core:mentions` gains no edge per quantity
  occurrence and the justification well-foundedness closure does not grow.
- **Term size.** No second full unit term per quantity, duplicated at every occurrence.
- **The codec, entirely.** No new `eigentt:Term` constructor, no D47 mirror arms, no
  `ontology::Value` carrier, no D1 Eigon-JSON form, no ESL printer or compiler change.

The normalised term keeps one unit, so `24 h` and `86400 s` still unify — which is what normalising
was for.

**And nothing new is needed, because the stated unit is not a datum to record — it is the raw input
to a normalisation, and the raw input is the prose.**

`24 h` → `86400 s` is a function application. The term holds the output; `enc:prose` holds the input,
verbatim, with `enc:span_start` and `enc:span_end`, and `enc:EncodedClaim` *requires* the link to it.
The conversion is deterministic — exact rationals, no search (D94) — so input, function and output
are all fixed. That is a complete record of the normalisation, and it needs no slot of its own.

**A distinction an earlier draft of this section collapsed.** There are two separate things in the
pipeline and only one of them is a record:

| | What it is | Where it lives |
|---|---|---|
| the stated unit | the **input** to a deterministic normalisation | `enc:prose`, already required |
| `931g` as gram or as standard gravity | a **choice** among felicitous parses | `enc:DecisionPoint` |

`enc:DecisionPoint` "records a structural-disambiguation choice (S4) … All candidates already
type-check, so this is auditable selection, not generation" (`encoding.esl:172-176`). It is the right
home for the `931g` sense, and the wrong frame for the stated unit: nothing was selected when an
author wrote `24 h`.

**Decided: no per-occurrence record.** The prose is the record, as the paragraph above argues.

A record `(stated_unit, span_start, span_end)` hung off the source span was designed and then
dropped. The one thing it added over `enc:prose` was making "which claims reported a dose in mg/kg"
a query over a field rather than a text search, and that is not a query this system needs. The
designs considered for identifying an occurrence — offsets, an ordinal, a term path, a value key, an
occurrence IRI in the term — went with it; none was needed once the question was.

**A vocabulary hazard the same evidence surfaced — and this document currently guarantees it.**
`931g` is g-force, not grams. The v1 vocabulary above is 7 base + 22 derived + 24 prefixes + the
SI-accepted list, which contains **gram and not standard gravity**. D95's split rule is "the longest
suffix that is a known unit", so against this vocabulary `931g` resolves to 931 grams — precisely
the silent mistyping this paragraph warns about.

**Decided (revised 2026-09-25): standard gravity is a unit, and `g` has two senses.**

- **The units layer gains `units:standard_gravity`**, symbol **`g_n`** — the symbol ISO 80000-3 and
  CODATA use for the standard acceleration of gravity — with dimension `s⁻²·m`, factor exactly
  `196133/20000` (9.80665 m/s², fixed by the 3rd CGPM, 1901) and no prefixes. Relative centrifugal
  force is therefore recorded as an acceleration: `931g` is `931 × 9.80665 m·s⁻²`.
- **The ambiguity is lexical, not in the units layer**, where every symbol stays unique and the
  strict stated form and the converter are unchanged. The prose surface `g` carries two senses,
  gram and standard gravity, as a word carries two synsets, and `RCF` is a surface of standard
  gravity alone. D95's parser seeds one chart item per sense; the ranker and the felicity gate choose,
  and conversion runs on the reading they select.
- **Prefixes still settle the common case.** Gram takes the 24 prefixes and standard gravity none,
  so `mg`, `μg` and `kg` have one sense; only a bare `g` has two.

**Why the refusal was withdrawn.** It was chosen as fail-closed, costing "nothing but coverage". It
costs more. A token the parser cannot interpret seeds nothing, so the sentence carrying it does not
parse at all: "the plates were spun at 931g for 2 h at 30 °C" loses `2 h` and `30 °C` with the
speed (D95). And the ambiguity is ordinary polysemy, which the chart already carries for nouns.

**The category, opened with a criterion.** Standard gravity is not on the SI's accepted list, so
admitting it opens "units science uses that the SI does not accept". It is opened by a rule rather
than symbol by symbol: **a unit enters when it is exactly defined and attested in the corpus.**
Standard gravity is both — defined by CGPM, and written `931g` in Nature's text of the WRN paper and
`931 RCF` in its PubMed Central manuscript (D96). `rpm`, in the same methods section, also meets the
rule and is the next candidate; °F, psi, mmHg and the calorie wait for attestation.

*Withdrawn — the earlier decision, kept as the record: "v1 REFUSES to split a `g`-suffixed numeral.
`931g` stays unparsed rather than becoming 931 grams." Its reasoning below still holds as fact; what
failed was its estimate of the cost.*

The two senses are not near-misses. Gram is a mass; standard gravity is an acceleration, `L T⁻²`,
so a wrong resolution types the wrong DIMENSION rather than the wrong magnitude.

The conversion is the cheap part: g₀ is *defined*, not measured, so it is `196133/20000` as a D94
rational — the clean side of this document's own distinction, unlike the dalton, which is measured
and carries a CODATA vintage as a documented exception. Prefixes disambiguate for free, since gram
takes the 24 and standard gravity takes none.

What it commits to is a CATEGORY, which the criterion above now governs. An earlier version of this
paragraph also said "`rpm` in the same sentence"; it is in the same methods section, not the same
sentence.

The second choice inside the admission is settled: RCF is conventionally reported as a
dimensionless multiple of g₀, and could have been a scale on the plain dimensionless unit, as `%`
and `ppm` are. It is recorded as an acceleration instead, because that is what a multiple of g₀
denotes, and a scale would have made `931g` and `931` the same quantity.

## "Normalised" names two operations, and only one is the kernel's

The word is used for both below, which invites a contradiction that is not there.

**Kennedy normalisation** sorts and reduces an exponent vector: `m · s⁻¹ · m → m² s⁻¹`. It converts
nothing. This is what the kernel primitive does, and it is why an open expression like
`Quantity (u · v⁻¹)` can be decided at check time.

**Base conversion** rewrites `km` as `m` and scales the magnitude by 1000. This happens once, at
ingest, when the stated form is turned into the normalised one.

Two consequences follow, and they answer a question that otherwise looks open:

- **The type index carries no scale**, because it is always in base units. `Quantity(km)` is not a
  type that arises; `Quantity(m)` with a magnitude of 1000 and a stated unit of `km` is.
- **The kernel never multiplies a magnitude.** Scale is consumed before anything reaches it, so the
  "size-increasing magnitude arithmetic" D94 puts out of v1 is genuinely out, not smuggled in by
  prefix folding.

**Decided: the chain carries base units only.** A type index is an exponent vector over the seven
base units and nothing else. `Quantity(g)` and `Quantity(km)` are not terms that arise; `Quantity(N)`
is not either, since a derived unit is a definitional abbreviation for `kg m s⁻²` and normalises to
that vector. The kernel therefore **restricts** rather than converts — an index that is not in base
form is rejected, not rewritten — and it never applies a scale.

That answers what happens to a term the parser did not build. An ESL source, the Rust API and an
institution result are all held to the same contract, so none of them needs an ingest normaliser and
none of them can smuggle in a scaled index.

**Convenience functions do the mapping.** An author writing `5 g`, or an API caller passing grams,
goes through a helper that converts to base units; the source it was called from shows `g`. These live in
the ESL elaborator and the Rust API — outside the kernel, on the same side of the TCB boundary as
D86's literal normalisation and the bridge's unit canonicalisation. Three carriers, one discipline,
and the kernel checks the result rather than performing it.

**Built** (`kernel/src/units/convert.rs`). The Rust API is `Vocabulary::from_layer(chain)` and
`convert(value, "mg/kg")`, reading symbols, factors, prefixes, the °C offset and kinds from the
units layer rather than restating them. The ESL form is `units:quantity(5, "mg/kg")`, resolved by the
compiler where the vocabulary is in hand into `(units:mk_quantity(c, pi) : units:Quantity(dim))`; it
is an elaboration form, not a chain constant, and its value must be an exact literal — `0.5` is
refused, `0.5r` accepted. The magnitude arithmetic lives in that module alone: `Rational` still has
none, so the checker cannot multiply a magnitude, which keeps D94's line. Two rules are decided
there:

- **The °C offset applies only to a bare `°C`**, the point reading. Inside a compound — `°C/min` —
  the reading is a difference, and °C converts as K with no offset: the rule above that a vector
  reading carries the vector unit.
- **Kinds come back beside the unit.** `rad/s` converts to `s⁻¹` with a plane angle in the
  numerator; `sr/rad` to `1` with a solid angle over a plane angle, which the unit, where both are
  `1`, cannot say (see "Kinds are metadata, not algebra").

## Prefixes fold in the normalised form and survive in the stated one

Whether `km` folds to `1000 m` looked like a standing question. The stated/normalised split answers
it: **the normalised form folds, the stated form keeps `km`**, and the same answer covers °C and
every non-SI accepted unit. One rule, three cases, no special pleading.

Two observations survive and belong to the implementation rather than the decision:

- **The SI itself does not fold, and the kilogram is why.** The base unit carries a prefix in its
  name; prefixes attach to the gram. Anything treating prefixes as pure scale special-cases the one
  base unit that disagrees, so the normaliser needs `kg` as the base and `g` as `kg` scaled by
  10⁻³ — the inverse of the naming.
- **Folding presupposes D94.** `1 Qm` folded is 10³⁰ m, precisely the magnitude that does not fit
  64 bits. Prefix folding is unimplementable without exact rationals.

## What this obliges of the parser

D93 is a kernel specification, but the reason it exists is that quantities in prose are currently
unrepresentable. The consuming side has obligations, and they start earlier than the grammar.
**D95 works them out**; this section states them and the constraints D93 places on them.

**Every numeral is routed out today.** `dcg::segment::is_nonprose` classifies a token as non-prose
when it "starts with a digit or carries no letters", and only `prose` units enter the DCG
(`enc:unit_kind`). So `37`, `0.56` and `931g` never reach the parser. Nothing about units can work
until that changes.

**Unit symbols are worse off than numerals — they are OOV, not routed out.** `ml`, `mM` and `°C`
contain letters, so `is_nonprose` passes them through as lexemes, and the lexicon has no entry for
any of them (`grep` over `ontologies/lexicon/` finds none). A unit symbol therefore presents as a
word the lexicon does not know. That is not merely unhelpful: `missing_lexeme == 0` is a tracked
gate, so admitting units without lexical entries breaks a measurement the project relies on. The
CNL corpus passes today only because it avoids units entirely.

**The tokenizer destroys unit syntax before the grammar sees it.** `tokenize` normalises
"em/en-dashes, slashes, and brackets" to spaces. So `mg/dL` becomes two tokens `mg` `dL`, and the
range `20–30%` becomes `20` `30%`. Compound units and ranges cannot survive tokenization as it
stands. Quantity recognition therefore belongs at or before tokenization — a span pre-pass, which
has precedent: the same function notes that "multiword forms are recovered by re-joining spans at
lookup time".

**The unit grammar is not the English grammar, and should not be made into one.** `mg/dL` is a
unit expression with its own syntax — that is what UCUM specifies — and it is not English. Making
the DCG parse it would be a category error, and would also mean enumerating prefix × unit
combinations as lexemes (24 × 29 before compounds). The right shape is a **separate unit parser
invoked on a recognised quantity span**, returning a `Unit` term; the DCG then sees the quantity as
a single atom of type `Quantity u`.

**A quantity is a third argument sort.** The grammar's arguments are entity senses today, and its
felicity gate gates to `Prop`. A quantity is neither: it is a term of type `Quantity u` appearing as
a PP object (`at 37 °C`, `for 1 h`) or a nominal modifier (`a 24 h incubation`). PP adjuncts are
already covered — the style guide lists `essential in MSI models` — so the extension is that a PP's
object may be a quantity rather than an entity, which is a typing change more than a structural one.

### How a quantity is represented in the grammar

**No new category is needed.** `lexicon:Cat` is already type-indexed:

```
cat_np : Set -> lexicon:Num -> lexicon:Cat        ⟦cat_np(T, _)⟧ = T
```

An NP denotes a value of its index type. So a quantity is an NP at a quantity type —
`cat_np(units:Quantity(u), n)`, denoting the `Quantity u` itself.

**The codomain must be `Set`, not `Type`, and this document said `Type` for several drafts.**
`Set = Sort 1` and `Type n = Sort (n+1)` (`kernel/src/esl/compile.rs:78-81`), so
`Quantity : Unit -> Type` would give `Quantity u : Sort 2`, which `cat_np : Set -> Num -> Cat`
(`ontologies/lexicon/lexicon-ontology.esl:270`) rejects — cumulativity runs upward only. That
`EigonPrimitive` infers to `Sort 1` establishes `Unit : Set`; it says nothing about the sort of
`App(Quantity, u)`, which its codomain fixes.

The alternative — widen `cat_np`'s index — is worse: `Cat` is "at Type 1, since `cat_n`/`cat_np`
store a `Set`", so widening pushes `Cat` to `Type 2` and forces `denote_cat`'s `cat_forall` arm
(`Π T : Sort 1`) to change with it.

**The composition RULES are unchanged; the LEXICON and the PP denotations are not.** An earlier
draft claimed the whole composition layer was unaffected. Three things in the code refute it:

- **`denote_cat` hard-codes `Entity` into every PP denotation** — `⟦cat_pp_arg(prep)⟧ = Entity`,
  `⟦cat_pp⟧ = Entity → Prop`, `⟦cat_measure⟧ = Entity → float`
  (`kernel/src/dcg/category.rs:71-104`). Since the felicity gate kernel-checks `sem : ⟦cat⟧`,
  `at 37 °C` as an argument PP fails there rather than composing. `cat_measure` is the exception
  that stays as it is: it denotes an **ordinal** scale, which admits ordering and no arithmetic, so
  measured quantities take a separate category rather than widening this one (D95, "Opaque scales
  and measured quantities are two sorts").
- **Every preposition entry is monomorphic at `Entity`** — `lexicon:at_arg` is
  `cat_np(lexicon:Entity, num_any)` with `sem_type = Entity -> Entity`
  (`ontologies/lexicon/closed-class.esl:1756-1763`), and `ontology:prep_at` is itself an
  entity-typed relation. So the widening is not merely categorial: a quantity-taking `at` needs a
  new relation, not a new index.
- **`type_subsumes` cannot relate a quantity index to anything** — it handles
  `(EigonClass, EigonClass)` through the subclass lattice and otherwise falls back to syntactic
  equality (`kernel/src/dcg/category.rs:434-439`). `Quantity u` is neither.

What survives is the narrower claim: the **combinatory rules** (application, composition,
type-raising) are untouched, because they are parametric in the category. The work is lexical and
in the `cat_pp*` denotations, and it is in scope.

What is genuinely new is narrower than it first appeared:

- **Lexical entries for unit symbols**, each carrying a quantity-typed `sem`.
- **Span recognition** before tokenization, since `mg/dL` and `37 °C` do not survive the current
  tokenizer. The recognised span becomes **one chart item** carrying a magnitude and, where present,
  a unit — a bare `0.56` is the same item with no unit, which is what keeps cardinality and
  quantities on one path rather than two.
- **The type-indexing of the PP categories.** `at 37 °C` and `at the promoter` are the same
  preposition over different index types, and reading the lexicon settled it: `⟦cat_pp_arg(_)⟧ =
  Entity`, `⟦cat_pp⟧ = Entity -> Prop` and `⟦cat_pp_than⟧ = Entity` all fix the object, so the
  widening is required. D95 records it.
- **`Num` is not a question.** A measure phrase is not a noun phrase: it is `MP`, and consumers
  subcategorise for it directly, so no agreement feature is chosen lexically. The earlier framing
  here, weighing `mass` against `name` (D70), does not apply. D95 records the mechanism.

**Unit symbols are ambiguous, and the existing machinery already handles that kind of problem.**
`931g` is g-force; `10 g` is grams. `M` is molar or mega. This is **polysemy, not a special case**:
a unit symbol is a lexeme carrying several senses, exactly as a noun carries several synsets, and
each sense is a different unit. It therefore routes through the machinery that already exists for
words — sense ranking, the felicity gate, and reading selection — rather than a bespoke
disambiguation rule. A wrong unit sense should be refused or down-ranked by the same path that
refuses a wrong noun sense.

**The parser emits both forms.** Given the stated/normalised split, recognising `37 °C` produces a
normalised magnitude and a stated unit, so the normaliser runs at parse time and not only at check
time. The °C rule lands here too, and v1's version is the weaker one: a bare °C is **assumed** to be a
point, and the difference reading is a recorded gap. Disambiguating `at` from `by` in the grammar is
what would close that gap; it is not what v1 does. (An earlier draft asserted both in one document.)

**The CNL style guide's first DON'T becomes wrong.** It currently instructs authors to drop inline
numbers — "the parser routes non-prose out; numbers are dropped, so a numeric claim is lost… state
the qualitative claim". Once quantities parse, that rule inverts for quantities while remaining
correct for test statistics. The guide anticipates this: it "is expected to drift as the grammar
grows; check a claim against the baseline before relying on it." D93 landing means that guide needs
a revision, not just an addition.

## The magnitude carrier — decided

`Quantity u` wraps a **magnitude**: a reduced rational coefficient and an integer exponent vector
over the declared constant set, `q × Π cᵢ^{eᵢ}` with `C = {π}` in v1. Not a float, and not a bare
rational.

That admits the transcendental unit category — angles, the parsec, atomic units — which a strict
rational would have excluded by consequence rather than by choice. It costs one more exponent
vector, which is the mechanism the primitive already has, and no new kind of machinery. D94
therefore supplies a *component* of a magnitude rather than the whole of it.

**And it costs one thing that was not on the table when the choice was made: `gcd` in the kernel.**
Everything else v1 stores is a terminating decimal — every conversion factor (eV→J, lb→kg, hour,
au, the prefixes), every measurement, and every binary64 value, since 2 divides 10. A decimal
carrier would therefore have sufficed for all of them and been canonical **by inspection**: strip
trailing zeros, no arithmetic. The angle factors are what break it — `37/180` and `1/32400` both
carry a factor of 3 and do not terminate — so the coefficient must be a general rational, and a
general rational must be reduced to compare structurally. One gcd per literal at admission, which
is what D94 now carries. Lean's own `Rat` pays the same price for the same reason.

### Why not the other two

`Quantity u` wraps a number, and D86 §4 records that the founding argument for `core:float` does not
survive while declining to take the pivot.

The choice is wider than float-versus-rational, and the third option is the one this document's
algebra section supplies:

| | transcendental units (angles, parsec, atomic units) | cost |
|---|---|---|
| `core:float` | representable, inexactly | inherits every D86 problem — non-reducing literals, the normalisation workaround, the version gate |
| strict `core:rational` | **out of v1 by consequence**, not by choice | simplest carrier |
| `q × Π cᵢ^{eᵢ}` over `{π}` | **in**, with equality provably decided | `Quantity` wraps a pair; the kernel gains one more exponent vector and no new kind of machinery |

The third buys angles, parsecs and atomic units for a mechanism the primitive already needs for
units, and keeps π in the kernel as a symbol rather than a number. Its cost is that a magnitude is
not "a number" — which also means D94 supplies a component of the magnitude rather than the whole
of it.

Two things make the exact readings better than they first look:

- **Every measurement is a finite decimal, hence exactly rational.** An instrument reports 4.21 mm,
  not √2 mm. Irrationality enters through *constants*, not measurements — and under the third
  option a constant is a declared symbol carrying an integer exponent in the magnitude's canonical
  form, not a `ConstRef` inside an expression. That is what keeps the comparison syntactic.
- **The kernel does not compute with magnitudes.** Institutions do. So the usual objection to exact
  rationals — repeated arithmetic blows up the denominator — does not apply where the value is
  stored and compared rather than accumulated.

### How a magnitude reaches a term — decided: as the arguments of `units:mk_quantity`

`units:Quantity (u : core:unit) : Set` has one constructor:

```
mk_quantity : forall (u : core:unit) => core:rational -> core:integer -> units:Quantity(u)
```

`37°` is `mk_quantity(r"37/180", 1)` at `Quantity(1)` — an angle is dimensionless. Constructors live flat in their data
type's namespace, so a bare `mk` would be `urn:eigenius:units:mk`, too generic for a namespace
that will grow.

**`u` is a parameter, so a quantity term does not carry its unit.** `mk_quantity(r"37/180", 1)`
inhabits `Quantity(u)` for every `u`, as `nil` inhabits every `List A`. The unit is fixed by the
type the term is checked against — a verb's signature, or an annotation. A quantity checked against
the wrong unit is refused on the chain path (`kernel/tests/units_layer.rs`). The type theory has no magnitude carrier: no `Exp`
variant, no `PrimitiveType`, no DataType, no ESL literal. `units::Magnitude` is computed at ingest
and lowered into these two arguments.

The pair is canonical by construction. A canonical rational and an integer each have one spelling,
and a quantity without π carries `pi = 0`. Equality is constructor-application equality on those
arguments, which the kernel already decides.

**The alternative was a `LitMagnitude` literal**: one canonical string in one slot, matching the
Rust type exactly. It would make a second constant an additive change, where this shape makes it an
arity change and a reseed. That cost lands where it belongs. A second constant needs a unit v1
excludes (the table above), and it is the point at which canonical equality goes from proved, by
Lindemann, to assumed, by Schanuel's conjecture. This document already requires that point to be
written down when it is reached.

**`units::Magnitude` is more general than this chain form**, in two ways the lowering must respect:

- `constants` is a map over `Constant`; the chain has one slot. Lowering is total while `Constant`
  has one variant, and becomes partial exactly when a second is added.
- The exponent is `i16`; `core:integer` is the 53-bit safe range. Lowering always fits. Raising a
  stored `pi` refuses a value outside `i16` rather than truncating it.

## Scope

**In.** The `Unit` kernel primitive: seven base symbols with a rational exponent vector — a group
element and nothing else, with quantity kinds recorded as metadata in the units layer (see "Kinds
are metadata, not algebra"); a declared constant set `{π}` with integer exponents; and
canonicalisation over both. A **magnitude** of `q × Π cᵢ^{eᵢ}`.
**Implicit Π was here and is deferred** (eigenius#261): an elaboration feature, per nanoda, not the
kernel change this paragraph once said;
`Quantity : Unit -> Set`; the SI content as chain ontology (7 base, 22 derived, 24 prefixes,
SI-accepted non-SI units — min, h, d, ha, L, t, Da, eV, au); conversion factors as exact rationals
(D94); a new `eigentt:Term` constructor carrying a unit value, `LitUnit`, with the codec arms it
obliges (the D47 mirror, `ontology::Value` as a canonical string — the rule `core:rational` follows,
see the implementation plan's "The carrier for `Unit` and `Magnitude`" — Eigon-JSON, the ESL printer
and compiler), a cost a primitive `Unit` pays and an inductive one would not; °C as a documented
extension with the point assumption. No stated-unit record: the prose is the record of what the
author wrote (see "Decided: no per-occurrence record").

An earlier draft put an **erased annotation node** in the term for the stated unit. That is out: a
datum every semantic operation must ignore is provenance, not meaning, and the encoding layer that
holds it is already required on every `enc:EncodedClaim`.

**Consequent work, specified in D95.** Span recognition before tokenization, the unit sub-parser,
quantity items in `seed_leaves`, unit-symbol lexical entries with senses, the measure-phrase category
and the subcategorised preposition entries that take it, and a revision of
`docs/method/controlled-english-style-guide.md` — whose first DON'T instructs authors to
drop inline numbers and becomes wrong for quantities once they parse.

**Out of v1.** AG-unification and unit-variable inference — including **`sqrt`-shaped signatures**,
since typing an application of `float<'u^2> -> float<'u>` requires solving `?u · ?u ≡ m²`. This costs
less than it appears: rational exponents were decided for *stated* units (`V/√Hz`, `MPa·√m`,
Manning's `s·m^(−1/3)`), not for root operations, and D52 is unblocked because an institution
computes a standard deviation however it likes and **declares** its result's unit. Also out: affine
units other than °C; logarithmic units (dB, Np, pH), which are also the only thing needing a second
constant; **intervals and ranges** (`15–18`, `20–30%`) — a follow-up that needs the
en-dash/hyphen distinction to separate a range from a catalogue number, and an interval type that
`formulas:` anticipates by naming IntervalArithmetic as a target institution — which is the kind of
thing `formulas:` is actually for, and the shape a later quantity projection into it would take.

**Prior art to align to rather than mint.** The BIPM SI Brochure as normative — the seven base units
are defined by fixed constants, which suits a formal system. Cite the **current** edition, not the
9th's 2019 printing: the four newest prefixes (ronna, ronto, quetta, quecto) are CGPM Resolution 3
(2022) and first appear in brochure V2.01, so "24 prefixes" is not a 2019 fact. ISO 80000 for quantity
definitions. QUDT as an RDF/OWL ontology that fits the graph model. UCUM as the code system for
parsing unit *strings* (`mg/dL`, `mm3`), which is what the grammar work will need.
