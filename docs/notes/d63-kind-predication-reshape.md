# D63 — Kind predication: bare mass/kind subjects as closed propositions

**Status:** design spec (proposed). Reshapes the grammar's treatment of bare mass/kind subjects.
Supersedes the deferred-quantifier / output-obligation model for the generic reading
(`docs/notes/d62-bare-plural-quantification.md`). Sibling of the alias-emission work
(`d63-document-preprocessing-scope.md` §3c), which stands unchanged — only the grammar's handling of
the `cat_n(C, mass)` entry it emits changes here.

## 1. The problem — a category error in the current model

A bare mass/kind subject parses today to an **open** parse carrying a `HoleKind::Quantification` hole:

```
"instability affects HeLa"   →  OPEN:  $quant$0_00(Instability, λx. affects(hela, x))
```

The hole `Q : Π(A:Set). (A→Prop) → Prop` ([`quant_hole_type`, lookup.rs:2703](../../kernel/src/dcg/lookup.rs#L2703))
is a *deferred quantifier*, and its documented discharge is a two-part **output obligation** — "bind a
quantifier **and** cite the literature `Reference` that warrants the generalization"
([`HoleKind`, lookup.rs:2845-2850](../../kernel/src/dcg/lookup.rs#L2845-L2850)).

This conflates two orthogonal axes:

- **Semantic completeness** — is the proposition well-formed? For a generic like *MSI contributes to
  cancers*, **yes**: it is a complete statement about the MSI *kind*.
- **Epistemic status** — is the claim warranted? That is a **grade + witness** question
  (`epistemic:declared` until a `reference:Citation` / observation / derivation elevates it).

The open-parse model puts an *epistemic* obligation (this claim wants a warrant) into the *syntax* (an
operator is missing). It is not missing an operator. The result is that a perfectly good proposition is
stuck in a resolution limbo, and the citation requirement is misfiled as a parser hole instead of a
grade.

## 2. The target — a closed kind-predication, graded `Declared`

```
"MSI contributes to cancers"  →  CLOSED:  contribute_to(kind_of(MSI), kind_of(Cancer)) : Prop
                                 grade:   epistemic:declared   (climbs on a witness)
```

The sentence parses to a **closed `Prop`**. Its subject (and here its object) is the *kind*, realized
as an entity. Justification is the ordinary Eigenius claim-grading, applied to a complete proposition —
never a parser hole.

### 2.1 Semantics: kind-level predication

*MSI contributes to cancers* is a predication **about the kind** (Carlson 1977, bare NPs denote kinds;
Chierchia 1998, nominalization). It is not a disguised `∃`/`∀` whose quantifier was dropped.

Eigenius is unusually suited to this because of **Resource-monism** ("everything is a `Resource`" —
`CLAUDE.md`). `MSI` is one resource with two facets:

- a **class** facet (`Set`) — instances are the tumours/samples exhibiting MSI (used when you quantify
  over instances: *each MSI tumour*);
- a **kind/entity** facet — the phenomenon itself, a referable individual you predicate over.

`contribute_to(kind_of(MSI), …)` uses the kind facet. Closed. Nothing is deferred.

### 2.2 The one missing primitive: the nominalization `kind_of`

The type system already has everything except a way to move from a class (a `Set`-value) to its
kind-individual (an `Entity`). Verbs and relations are `Entity`-typed
(`affects`/`compound`/`prep_* : Entity → Entity → Prop`,
[ontology.esl:43-54](../../ontologies/ontology/ontology.esl#L43)); a class `EigonClass(C) : Set` cannot
fill an `Entity` slot ([`type_subsumes`, category.rs:261-266](../../kernel/src/dcg/category.rs#L261) only
coerces `EigonClass ≤ EigonClass` by subclass). The gap the grammar map already flagged: *kinds stay at
`Set`; there is no reify-as-`Entity` path.*

Add exactly one axiom — Chierchia's down operator `∩`, a class → its kind-individual:

```
// ontologies/ontology/ontology.esl
// kind_of(C) — the kind C, realized as an individual (Chierchia ∩ nominalization). A class is a
// Set-value (EigonClass : Set), so this maps a Set to the Entity that IS that kind. Lets a bare
// kind/mass NP be a verb argument: "MSI contributes to cancers" → contribute_to(kind_of(MSI), …).
axiom ontology:kind_of : Set -> lexicon:Entity
```

This mirrors the existing `Set`-argument axioms (`subclass_of : Set -> Set -> Prop`,
`compound_kind : Entity -> Set -> Prop`, [ontology.esl:32,47](../../ontologies/ontology/ontology.esl#L32)),
so it needs no new kernel type machinery. `kind_of(EigonClass(C)) : Entity` type-checks by the axiom's
return type — **`type_subsumes` is left unchanged** (no `Set ≤ Entity` hack). Because the term is
axiom-headed and canonical, `kind_of(MSI)` is a stable individual (the same entity every occurrence) —
good for identity/dedup in the reasoning layer.

Why a nominalization operator and not a kind-accepting predicate overload: `kind_of` is the **single,
general** mechanism. The alternative — giving every verb a `Set`-accepting variant — multiplies every
predicate's type. One operator reuses the entire `Entity → … → Prop` relation vocabulary as-is.

## 3. The grammar change — one unified shift for mass and plural

The bare mass **and** plural shifts (formerly deferred-quant NPs carrying the `Quantification` hole) are
one committed, type-raised rule, [`LexicalIndex::kind_raised_nps`](../../kernel/src/dcg/lookup.rs) (core-en's
`bnp` is likewise one rule over `pl-or-mass` — §7.4):

| input | today | reshaped |
|---|---|---|
| `cat_n(C, mass)` (bare, reuse `a`) | raised NP, sem `λV. Q(C, λx. V(x))` — hole | **raised** `S/(S\NP)` (subj) / object raise, sem `λV. V(kind_of(C))` |
| `cat_n(C, pl)` (bare, reuse `these`) | raised NP deferred-quant + `cat_kind` (copula) | as above (subj/obj) **and** `cat_kind` (copula) retained |
| `cat_n(Σx:C. R, pl/mass)` (bare COMPOUND) | (open, or gapped) | raised, base-indexed `S/(S\NP_C)`, sem `λV. V(kind_of(Σx:C. R))` |

- The shift **builds the raised NP directly** — it takes the existential determiner's (`a` / `these`)
  subject- (`fwd`) and object- (`bwd`) raised CATEGORY, substitutes the noun's **base class** for the
  determiner's type variable, and attaches the kind sem `λV. V(kind_of(t))` — it does **not** call
  `apply`. Building directly is **load-bearing for refined (compound) nouns**: routing `cat_n(Σx:C.R, …)`
  through `apply` hits the GQ witness-projection (`DetRefine`, [parser.rs:448](../../kernel/src/dcg/parser.rs#L448)),
  which wraps the sem as `λv. det(t)(λz. v(Fst z))` and yields the ill-typed `Fst(kind_of(Σ))` — a kind
  nominalizes the *whole* type, it does not project witnesses. Indexing the raised category by the base
  `C` (`C ≤ Entity`, in the subsumption lattice) lets it fill a verb slot; nominalizing `kind_of(Σx:C.R)`
  keeps the compound's content. (This was the "nucleotide repeat regions" / "gene genes" bug.)
- **Type-raising is load-bearing, not incidental.** A raised `S/(S\NP)` is not a `cat_np`, so it fills a
  verb argument slot by application but **cannot** feed the named-entity compound rule (which keys on
  `is_ctor(cat_np)`). This keeps a noun's prenominal reading to the single kind classifier
  `compound_kind(x, C)` (from its `cat_n`) with **no** spurious `compound(x, kind_of(C))` duplicate (§7.5).
  We deliberately diverge from core-en here — core-en's `bnp` yields a *plain* `np` because core-en has
  no noun-noun compound rule to misuse it; we do, so we raise.
- `*a MSI` / `*two MSI` still fail — the underlying `cat_n` stays `mass`, so no real determiner composes.
- The alias emitter's `cat_n(C, mass)` output is unchanged; it now yields a closed reading, not an open one.

### 3.1 The copula case is already anticipated

Kind-**kind** predication (*Genes are cell lines* → `subclass_of(Gene, CellLine)`) already has its
relation and its `are_kind` consumer; [ontology.esl:27-31](../../ontologies/ontology/ontology.esl#L27)
explicitly notes it "awaits kind subjects." The two readings coexist cleanly:

- kind **as entity**, verbal predication → `kind_of(C) : Entity`, fills a verb arg (this note);
- kind **as class**, copular subsumption → `subclass_of(C, D)`, via `are_kind` (`cat_kind`, existing).

Both are closed `Prop`s. `is_a : Entity -> Set -> Prop` ([ontology.esl:25](../../ontologies/ontology/ontology.esl#L25))
already covers instance-membership (*HeLa is a cell line*), and composes: *MSI is a dysfunction* →
`is_a(kind_of(MSI), Dysfunction)`.

## 4. Justification is the grade, not a hole

The three warrant routes the user named map exactly onto the native epistemic grades
(`ontologies/reflection/`, `epistemic:declared/derived/observed/verified`) applied to the **complete**
proposition:

| warrant | grade / witness |
|---|---|
| (none yet) | **`epistemic:declared`** — asserted, the honest default the parser emits |
| literature reference | attach a **`reference:Citation`** (a `reflection:DeclaredResource`, CiTO `cites_as_evidence` — [reference.esl:147,172](../../ontologies/reference/reference.esl#L172)) → cited/attested |
| observation | **`epistemic:observed`** |
| computational derivation | **`epistemic:derived`** |
| proof / kernel-checked | **`epistemic:verified`** |

The parser's job ends at a closed `Declared` `Prop`. Turning it into a graded, witnessed claim — and
climbing the grade when a `reference:Citation` / observation / derivation is attached — is the existing
reasoning/proposition layer (D39, the `reasoning` skill), the same machinery every claim uses. The
generic is not special.

## 5. What this dissolves, and what it keeps

**Dissolves** (for the generic reading): `HoleKind::Quantification`
([lookup.rs:2850](../../kernel/src/dcg/lookup.rs#L2850)), `deferred_quant_subj_sem` /
`deferred_quant_obj_sem` / `quant_hole_type` / the `$quanthole$` sentinel, and the D62 two-part "output
obligation" (`bind quantifier + cite`). The citation half becomes a grade; the quantifier half becomes a
committed `kind_of` (no choice was ever needed for the kind reading).

**Keeps** — genuine quantifier ambiguity. A bare plural *can* have an existential or universal reading
when context forces a non-generic, episodic one (*genes were mutated in the assay* ≈ `∃`). These stay as
**committed** alternative parses (a real `∃`/`∀`/`GEN` operator, closed), ranked **below** the
kind-predication default for scientific-register generics. The point is that force, when determinate, is
*committed*, not parked in a free variable. (Whether to retain any deferred hole for a truly ambiguous
residue is Open Question 5.2.)

## 6. Scope & phasing

- **Phase A (kernel + grammar) — DONE (`2026-07-03`, mass **and** plural; in-memory, no reseed):** the
  `kind_of` axiom; the unified `kind_raised_nps` (§3) for bare mass + plural, including refined/compound
  nouns (the `DetRefine`-bypass, §7.4). Full suite + fmt + clippy green. The load-bearing change.
- **Phase B (cleanup) — partly done:** the deferred-quant *sems* are removed (nothing produces a
  `QUANT_SENTINEL`). The quantification hole **carrier** (`freshen_quant`, the per-span registration,
  `HoleKind::Quantification`) is left INERT, its retirement gated on the corpus re-measure confirming
  committed-only suffices (§7.2). Then update `d62-bare-plural-quantification.md`.
- **Phase C (grade attachment) — DONE (Declared floor):** parsed props enter the reasoning layer as
  `Declared`, built by `eigenius-reasoning::grade` (`ClaimGrader` + `DeclaredClaimGrader`). A closed
  `Prop` → a **3-resource claim cluster** (declaring `reflection:DeclaredResource` with
  `canonical_proposition` + its `DeclarationTrace` + the `reasoning:ReasoningSentence` with a
  `JustifiedBy.declared` certificate) → committed → the D39 gate returns `Holds`. Composed into the
  document→claims path by `eigenius-reasoning::ingest` (`DocumentIngestion` / `InProcessIngestion`),
  witnessed end-to-end (`instability affects HeLa` → `affects(kind_of(Instability), hela)`, `Holds`).
  *Integration was net-new wiring, not "largely existing": it was the first consumer of a term-level
  resource individual in a proposition, which required completing the D47 codec (`EigonResource ↔
  ConstRef`).* **Remaining:** the `reference:Citation` witness grade-climb (§4 row 2) — `Warrant` is
  `#[non_exhaustive]` for it.

## 7. Open questions

1. **`cat_np(Entity)` vs `KindOf(C) ≤ Entity`.** `kind_of : Set → Entity` loses the class index (a
   kind fills any `Entity` slot, not narrowed to *C*-kinds). If selectional restriction on kinds is
   needed (a verb wanting a *disease* kind), introduce `KindOf : Set → Set` with `KindOf(C) ≤ Entity`
   and `kind_of : Π(A:Set). KindOf(A)`. Start with plain `Entity` (simplest, matches the loose
   selection verbs like *contributes/affects*); refine only if a real selectional case appears.
2. **Deferred residue.** Keep `∃`/`∀`/`GEN` as committed ranked alternatives only, or retain one
   deferred hole for a genuinely ambiguous residue? Lean committed-only; re-open if measurement shows
   over-generation.
3. **`GEN` vs bare kind-predication.** Is `contribute_to(kind_of(MSI), …)` (kind-predication) the final
   form, or a surface for an exception-tolerant `GEN[MSI(x)][contribute_to(x, …)]`? Both are closed;
   kind-predication is the economical default. If instance-level truth-conditions are needed, `GEN` is a
   committed dyadic operator over the class, not a hole.
4. **Mass vs plural unification — RESOLVED (`2026-07-03`).** One rule, `kind_raised_nps`, serves both
   (parameterized by the determiner reused, `a` vs `these`, and the number gate) — matching core-en's
   single `bnp` over `pl-or-mass`. Resolving it exposed the **composed-compound bug**: a bare-plural
   compound ("nucleotide repeat regions", "gene genes") is a *refined* noun `cat_n(Σx:C.R, pl)`, and
   routing it through `apply` hit `DetRefine`'s witness-projection (`λv. det(t)(λz. v(Fst z))`), which
   for the committed kind sem produces the ill-typed `Fst(kind_of(Σ))` (a kind nominalizes the whole
   type; it doesn't project witnesses). Fixed by building the raised NP directly (§3) — index by the base
   `C`, nominalize `kind_of(Σx:C.R)`. (Earlier misdiagnosis: I first blamed `Σ ⊀ Entity` subsumption and
   added a `type_subsumes` arm — it changed nothing and was reverted; the category was never the problem,
   the sem was.)
5. **Spurious modifier reading (found and RESOLVED in Phase A, `2026-07-03`).** A first Phase-A cut
   emitted the kind term as a plain `cat_np(Entity, sg)`. That leaked a spurious prenominal reading: "MSI
   cell lines" parsed **both** as the intended `compound_kind(x, MSI)` (from the `cat_n`) **and** as
   `compound(x, kind_of(MSI))` (the plain `cat_np` feeding the named-entity compound rule) — a
   near-synonymous duplicate on the actual corpus ("MSI cell lines/tumours/cancers", "MMR genes"). Root
   cause: `cat_np` had silently conflated "entity-denoting argument NP" with "proper name that can
   name-compound", and a kind is the first entity-NP that is not a name. **Fixed structurally** by making
   the shift **type-raised** (§3) rather than a plain `cat_np` — the resolution the bare-plural shift
   already uses. A raised `S/(S\NP)` fills verb argument slots but is not a `cat_np`, so the compound rule
   cannot fire on it; the modifier drops back to the single `compound_kind` reading. Guarded by
   `abbreviation_emission_keys_on_ontological_kind` (asserts the mass modifier has exactly one reading,
   `compound_kind`, no `kind_of`). *(Rejected alternative: ranking `compound_kind` above `compound` — it
   would leave the spurious parse in the forest, modelling a false ambiguity rather than eliminating it,
   contrary to the Eisner-NF stance on spurious/derivational ambiguity.)*

## 8. Verification (witnesses to hold)

1. `MSI contributes to cancers` (schematically, on the demo: `instability affects HeLa`) → a **CLOSED**
   `Prop` `affects(kind_of(Instability), hela)`, kernel-gated, **no open holes** — replacing today's
   `$quant(Instability, …)` open parse.
2. The closed prop commits at `epistemic:declared`; attaching a `reference:Citation` (CiTO
   `cites_as_evidence`) is accepted and recorded as the warrant (grade climb), through the kernel gate.
3. Regressions hold: `*a instability` / `*two MSI` still fail (mass); `a instability cell line …` still
   → `compound_kind(x, Instability)` (classifier unchanged); `Genes are cell lines` still →
   `subclass_of(Gene, CellLine)` (copula unchanged).
4. `cargo build/test`, `fmt`, `clippy` clean; the `kind_of` axiom loads and validates on the real
   ontology (no bootstrap drift beyond the added axiom).
