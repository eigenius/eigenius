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

**Taking a square root does not force this.** Kennedy types `sqrt : float<'u ^ 2> -> float<'u>` —
the input is constrained to be a perfect square, so a root never produces a fractional exponent.
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

**Implicit unit arguments are admissible, under one condition.** Writing `mean {u} [x,y,z]` and
having `u` inferred is implicit-argument solving, and it stays first-order — hence within the
existing `implicit(…)` machinery that `justification:Grounds.app` already uses — precisely when
**every implicit unit variable appears alone as the index of at least one explicit argument's
type**. Then solving is `Quantity ?u ≡ Quantity m`, which is syntactic.

A signature that violates the condition — an implicit `u` occurring only inside a product, as in a
polymorphic constant `c : {u} -> Quantity (u * u)` — requires solving `?u * ?u ≡ m * m`, which is
AG-unification and therefore outside v1. Such a signature is rejected rather than half-supported;
writing the unit explicitly as a Π parameter is always available.

## What is trusted, and how little

**Only the seven base units.** F# puts the algebra in the language and the SI content in a library —
its PowerPack "declares all of the SI base and derived units". Kennedy is explicit that derived units
are definitional: `[<Measure>] type N = kg m/s^2`, and "`N` and `kg m/s^2` mean exactly the same
thing".

So the 22 named derived units and the 24 prefixes are **abbreviations that normalise away**, not
additional primitives. The trusted surface is:

- seven base-unit symbols (second, metre, kilogram, ampere, kelvin, mole, candela),
- a rational exponent vector over them,
- a quantity-kind tag (see "Dimension is not the whole of a unit"),
- a canonicalisation: sort, reduce the fractions, drop zero exponents.

That is the whole TCB addition. The SI *content* lives in chain ontology where it is authored,
reviewed and replaceable without touching the checker.

The kind tag is the one part that is a tag rather than an algebra, and it is worth being explicit
that it grows: `rad`, `sr` and plain-dimensionless are the v1 set, and admitting a new kind is a
vocabulary edit in the units layer, not a checker change — provided the normaliser treats kinds
opaquely and only compares them for equality.

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

**Decided.** A unit carries a **quantity kind** alongside its dimension, and kind participates in
the normal form, so `rad` and `sr` do not unify even though both have dimension 1. This is QUDT's split between `qudt:QuantityKind` and
`qudt:QuantityKindDimensionVector` — and QUDT already carries this exact case: `PlaneAngle` and
`SolidAngle` are distinct quantity kinds sharing one dimension vector (`A0E0L0I0M0H0T0D1`). One more
reason to align rather than mint.
`%` and `ppm` are not kinds but **scales** on the plain dimensionless unit, which is what they
actually are.

**Angle as an eighth base dimension was considered and rejected.** It distinguishes `rad` from `sr`
elegantly (`sr = rad²` falls out, though no edition states it) but breaks `s = rθ`: with angle dimensional, arc length would come
out as metre·angle rather than metre, and recovering it requires introducing a constant θ₀ = 1 rad
throughout. The SI keeps angle dimensionless for this reason, and the kind axis gets the same
distinction without the cost.

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

**Load order.** After `core`, which supplies the primitive types the magnitude rests on. Before
`formulas` and `statistics`, both of which would reference it — `formulas:Operator` signatures for
unit-typed operators, and D52 quantities for their units. In the current bootstrap sequence
(`kernel/src/bootstrap/mod.rs`) that places it between `runtime` and `formulas`; today `formulas`
loads eighth and `statistics` twelfth, and neither references units yet, so inserting the layer is
additive rather than a reordering.

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

**Units only in `FormulaTerm`.** Its constructors are `Var | LitFloat | OpRef | App | Lam | Pi`
(the ontology's own prose says it "mirrors `Exp::Var / App / Lam / Pi` one-for-one", which
undercounts by two). A unit expression *is* a product of powers, so it fits that fragment
as an `App` spine over base-unit constants, and `formulas:Operator` already carries typed signatures
with declared associativity and commutativity. But FormulaTerm is a chain-level projection that
institutions read; putting units only there leaves the kernel unable to check them. FormulaTerm
should carry the *projection* of a unit, not its definition.

## Affine units are an extension, not part of v1

°C is the only affine unit in the SI — the other 21 named derived units are multiplicative and sit
inside the group unchanged. Fahrenheit and Rankine are non-SI and out of scope.

Affine units are **outside Kennedy's theory**: the free Abelian group is multiplicative, and the
paper addresses offsets nowhere. So °C needs its own construction.

**v1 normalises °C to K at ingest, under an explicit assumption.** The assumption is necessary
because °C is used for two different things, and the SI says so: a temperature *point*
(`heated to 85 °C` → 358.15 K) and a temperature *difference* (`rose by 5 °C` → 5 K, **not**
278.15 K, because the degree Celsius is equal in magnitude to the kelvin). Nothing in the unit
distinguishes them; the disambiguator is in the prose — "to" versus "by".

So affine handling is partly a *grammar* concern, not purely a units one. v1:

- assumes the **point** reading for a bare °C, which dominates in methods prose (`85 °C`, `37 °C`),
- records the **difference** reading as a known gap until the grammar can disambiguate,
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
- **Irrational but exactly expressible** — 1° = π/180 rad. No rational represents it. The factor is
  a symbolic expression with π as a constant — **which `formulas:` does not currently provide.** Its
  operator catalogue is `add sub mul div pow neg exp log sin cos tan sqrt abs eq lt le derivative`
  over `types:Real/Int/Bool`; there is no π, e or c. Adding the mathematical constants is scope, not
  an existing affordance.
- **Measured, carrying uncertainty** — the dalton is 1.660 539 068 92(52) × 10⁻²⁷ kg (CODATA 2022),
  a measurement rather than a definition. It is also the case in point: an earlier draft of this
  document quoted the CODATA 2018 value, superseded in 2024, while arguing in this very paragraph
  that a measured factor must not masquerade as exact. A measured constant carries a *vintage* as
  well as an uncertainty, and the chain must record both. A conversion factor that is itself a measurement belongs to D52, not
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

**What remains open is where the stated unit sits**, and the candidates trade against each other.

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

**Decided: in the term.** The question was whether a single proposition carries quantities stated in
different units, and the WRN methods material answers it. Of 240 sentences, **nine carry two
distinct units and one carries three**:

> "The flask was placed on a rotator and incubated at 37 °C for 1 h."
>
> "…the plates were spun at 931g for 2 h at 30 °C."

One claim, two or three quantities, unrelated units. A single resource-level property cannot record
them, so the stated unit rides with the quantity in the term and the resource-property option is
out.

**The consequence is accepted, not evaded.** Stated units are not queryable until EigenQL can decode
terms. "Which claims reported a dose in mg/kg" is not a query today. That is the same limitation the
justification vocabulary already records for warrant projections, and the same answer applies: it is
a Rust-API question now and an EigenQL one when terms become decodable. Recording an unqueryable
fact correctly is better than recording a queryable one that cannot represent the data.

**What this costs**, beyond the node itself:

- **Reference edges.** The stated unit names unit resources by IRI, and `core:mentions` projects a
  term's internal references into the graph. So every quantity occurrence adds an edge to the units
  layer, every claim carrying a quantity acquires a dependency on it, and the justification
  well-foundedness check — transitive closure over `core:mentions` — grows accordingly.
- **Term size.** The stated unit is a second full unit term per quantity, duplicated at every
  occurrence, in a system where terms are stored and re-checked.
- **The codec.** A new `eigentt:Term` constructor, encode/decode arms in the D47 mirror, a carrier
  in `ontology::Value`, the D1 Eigon-JSON form, and the ESL printer and compiler.

None of these is an objection; all of them are scope, and an earlier draft named only the node.

**A vocabulary hazard the same evidence surfaced — and this document currently guarantees it.**
`931g` is g-force, not grams. The v1 vocabulary above is 7 base + 22 derived + 24 prefixes + the
SI-accepted list, which contains **gram and not standard gravity**. D95's split rule is "the longest
suffix that is a known unit", so against this vocabulary `931g` resolves to 931 grams — precisely
the silent mistyping this paragraph warns about.

So v1 must do one of two things, and it must say which: admit standard gravity (and the
relative-centrifugal-force reading) as a named sense so the ranker can choose, or **refuse** to
split a `g`-suffixed numeral rather than resolve it wrongly. Refusing is the fail-closed option and
is the default until the sense exists.

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
  `at 37 °C` as an argument PP fails there rather than composing.
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
- **Whether prepositions constrain their object's type.** `at 37 °C` and `at the promoter` are the
  same preposition over different index types. If `at` is already polymorphic in its object's `T`,
  nothing changes; if it is constrained, it needs widening. This is a question about the existing
  lexicon, answerable by reading it.
- **Which `Num` a quantity carries.** The feature is syntactic and erased by ⟦·⟧, so it only routes
  agreement. `mass` (used bare as an argument, no determiner) is the closest existing fit — `24 h`
  takes no article the way `MSI` does not — but `name` (D70, proper name of a kind) is also bare.
  Reusing `mass` risks agreement behaviour a quantity should not have; a distinguished variant may
  be cleaner. Small, but it should be decided rather than defaulted.

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

## Open questions

### The magnitude's numeric type

`Quantity u` wraps a number, and which number is D86's to settle, not this document's: §4 records
that the founding argument for `core:float` does not survive and points at an exact rational without
taking the pivot. A float-backed quantity inherits every D86 problem — non-reducing literals, the
literal-normalisation workaround, the Lean-version gate. An exact-rational one inherits none.

Two things make the rational reading better than it first looks:

- **Every measurement is a finite decimal, hence exactly rational.** An instrument reports 4.21 mm,
  not √2 mm. The irrational quantities in science are *constants* (π, e, c in natural units), and
  those are symbolic. `formulas:` does **not** carry them today — no π, e or c in the operator
  catalogue — so admitting them is scope.
- **The kernel does not compute with magnitudes.** Institutions do. So the usual objection to exact
  rationals — repeated arithmetic blows up the denominator — does not apply where the value is
  stored and compared rather than accumulated.

What would decide it: whether any kernel-side operation needs to *reduce* a magnitude rather than
compare it. D94 argues none does.

## Scope

**In.** The `Unit` kernel primitive (seven base symbols, a rational exponent vector, a quantity-kind
tag, and canonicalisation);
`Quantity : Unit -> Set`; the SI content as chain ontology (7 base, 22 derived, 24 prefixes,
SI-accepted non-SI units — min, h, d, ha, L, t, Da, eV, au); conversion factors as exact rationals
(D94); the FormulaTerm projection; °C as a documented extension with the point assumption; an erased
annotation node so a quantity can carry its stated unit without that unit entering definitional
equality.

**Consequent work, specified in D95.** Span recognition before tokenization, the unit sub-parser,
quantity items in `seed_leaves`, unit-symbol lexical entries with senses, the `Num` decision, and a
revision of `docs/method/controlled-english-style-guide.md` — whose first DON'T instructs authors to
drop inline numbers and becomes wrong for quantities once they parse.

**Out of v1.** AG-unification and unit-variable inference; affine units other than °C; logarithmic
units (dB, Np, pH); **intervals and ranges** (`15–18`, `20–30%`) — a follow-up that needs the
en-dash/hyphen distinction to separate a range from a catalogue number, and an interval type that
`formulas:` already anticipates by naming IntervalArithmetic as a target institution.

**Prior art to align to rather than mint.** The BIPM SI Brochure as normative — the seven base units
are defined by fixed constants, which suits a formal system. Cite the **current** edition, not the
9th's 2019 printing: the four newest prefixes (ronna, ronto, quetta, quecto) are CGPM Resolution 3
(2022) and first appear in brochure V2.01, so "24 prefixes" is not a 2019 fact. ISO 80000 for quantity
definitions. QUDT as an RDF/OWL ontology that fits the graph model. UCUM as the code system for
parsing unit *strings* (`mg/dL`, `mm3`), which is what the grammar work will need.
