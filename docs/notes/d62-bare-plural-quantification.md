# D62 — bare-plural NPs as deferred-quantification holes, justified by a literature reference

> **SUPERSEDED (2026-07-04) by the D63 kind-predication reshape**
> (`docs/notes/d63-kind-predication-reshape.md`). This note's central mechanism — a bare plural becomes
> an argument NP carrying a **deferred `Quantification` hole** whose discharge is quantifier-binding +
> a warranting citation — was retired in the reshape's Phase B. A bare plural (like a bare mass noun,
> Carlson 1977) now **commits to its kind**: it parses to a CLOSED `kind_of(t) : Entity` predication
> (`LexicalIndex::kind_raised_nps`), no hole. A generic is a complete proposition; its warrant belongs
> on the claim's **grade**, not a parser hole. The full-UMLS re-measure confirmed OPEN=0. The material
> below is retained for the design rationale and the core-en/`bnp` grounding, which the reshape reuses.

*Design note. Fixes how a **bare plural** common noun (`genes affect cells`) becomes an argument NP,
and what its underspecified quantification means in our typed kernel. Grading: **Derived** = grounded
in current code / the core-en reference grammar; **Declared** = a design choice this note makes.*

## 0. The gap (Derived)

After the felicity-gate OOM fix ([[axiom_env_fullscan_oom]]), the WRN page is grammar-limited, and the
**highest-leverage** gap is **bare-plural NP arguments**. Witnessed (small lexicon, plural-aware
lemmatizer; full lexicon agrees):

| clause | parses |
|---|---|
| `these genes affect HeLa` (determiner + plural) | ✓ |
| `genes affect HeLa` (bare plural subject) | **✗** |
| `HeLa affects genes` (bare plural object) | **✗** |
| `genes are large` (bare plural + predicate adj) | **✗** |

A plural common noun `cat_n(C, pl)` is number-refined and gets a `kind_subject` edge, but has **no
type-shift to an argument `cat_np`** — only determiners produce argument NPs. Bare plurals (and mass
nouns) are pervasive in scientific prose, so this blocks the most basic clause shape.

## 1. The reference grammar already has the rule (Derived — core-en)

core-en (`references/openccg/grammars/core-en/unary-rules.xsl:80`) has a dedicated **`bnp`
("bare NP") unary type-changing rule**:

- **Shape:** unary `n → np` (a type-change, *not* a determiner application).
- **Restriction:** the argument is `num = pl-or-mass` — **plural or mass, never bare singular count**.
  This is exactly why `genes`/`water` shift but `*gene is a vulnerability` does not (`WRN`-as-CN bare
  singular is genuinely ungrammatical; that failure is the separate *gene-symbol-as-proper-noun*
  lexicon-modeling issue, not this rule).
- **Semantics:** it sets the determiner to **`<det>nil`** — i.e. it **declines to quantify**. `nil` is a
  pure marker; **nowhere in core-en is it resolved to ∃/∀/generic** (even `one` and the cardinals carry
  `det=nil`). Quantificational force is deferred to a downstream semantic/pragmatic layer.

So the mechanism is settled — a unary shift gated on plural/mass — and core-en deliberately leaves the
**quantifier underspecified**.

## 2. Rendering `det=nil` in a typed kernel: a deferred *quantification hole* (Declared)

Unlike core-en (a feature-structure grammar that can carry `nil` inertly), our NP must denote a
concrete `(Entity→Prop)→Prop` GQ for the clause to gate to `Prop`. We **cannot** silently default to
∃ — that is the premature commitment the core-en grounding warns against, and it would fix the logical
form of every bare-plural scientific claim without warrant.

**Decision:** render `det=nil` as a **deferred quantification hole** on the D64 open-parse carrier
(`kernel/src/dcg/lookup.rs`, `HoleKind` — today `EntityRef`; `ProofObligation` is the planned arm).
The bare-plural shift produces:

- **cat:** the argument NP (subject type-raised `S/(S\NP(C,pl))`; object the in-situ raise), exactly
  as `these`/`some` yield — reuse the determiner cat machinery (the loaded plural-existential
  determiner `these`), not a hand-rolled cat. The shift dispatches subject vs object by the
  determiner's post-`cat_forall` body head (`fwd` ⇒ subject, `bwd` ⇒ object) to pick the matching sem.
- **sem:** the determiner's quantifier replaced by the hole `Q` (type `Π(A:Set). (A→Prop) → Prop`):
  - subject `λA. λV. Q(A, λx. V(x))`  (from `exists_sem`)
  - object `λT. λTV. λsubj. Q(T, λx. TV(x, subj))`  (from `obj_exists_sem`)

  applied to the noun class `C` → subject NP sem `λV. Q(C, λx. V(x))`. The clause `genes affect cells`
  becomes `Q₁(Gene, λg. Q₂(Cell, λc. affect(c, g)))` carrying **two** quantification holes; each `Q`
  binds to a generic neutral of quantifier type (the higher-order analogue of how `EntityRef` holes
  bind to an `Entity` neutral), so the clause is a felicitous **open** `Prop`.

  > **η-expansion is load-bearing (Derived, slice 2).** The scope MUST be the λ `λx. V(x)`, not the
  > rigid VP `V` passed whole. An *opaque* hole `Q` applied to a rigid `affects(hela) : Entity→Prop`
  > against `Q(Gene, ·)`'s expected `Gene→Prop` needs contravariant arrow subtyping the kernel does
  > not do, and is **rejected**. The η-redex `λx. V(x)` moves the `x:Gene`-against-`Entity` subsumption
  > to the λ body (argument position) — exactly how the *concrete* `∃` does it (`∃x:A. V(x)`). The
  > de-risk probe was initially too weak (a λ-scope hid this); it now uses a subclass restrictor.

A bare plural is therefore an **`Open` parse**, not a closed one — faithful to `det=nil`.

## 3. The hole discharges to a literature reference (Declared — the load-bearing move)

The quantificational force of a scientific bare-plural generic — "mutations cause cancer",
"WRN-deficient cells require POLθ" — is an **empirical generalization whose warrant is the cited
evidence**, not a free logical pick. So the quantification hole's discharge is a **justification act
anchored in a literature `Reference`**:

- It is an **output obligation** in the [[d62-encoding-output-contract]] sense (§3–4) — the
  `ProofObligation` family, *not* the internal-resolution `EntityRef` family. It **survives** parsing
  and feeds the obligation set.
- Its **witness kind is grounding** (contract §4: *grounding/discovery → retrieval (D43) → an anchored
  fact*). Discharging it = binding `Q` to a quantifier **and** citing the `Reference` that warrants the
  generalization (the global work, per [[reference_ontology_modeling]]: `Reference` = the work,
  `Citation` = its use; real DOIs/PMIDs, never fabricated).
- Discharge **raises the proposition's grade** from Declared (parses) toward an anchored/Derived claim
  — the grade ladder applied to encoding (contract §7).

This is the same justification-logic pattern as the measurement-adverb arm
([[d62-adverb-semantics-decision]] §4a): a Declared proposition *carrying* an obligation that a
downstream institution discharges. Here the discharger is **grounding/citation**, and it directly
serves D61 faithful-encoding: the generalization is grounded in *discovered, cited* evidence rather
than an assumed quantifier.

## 4. Why not the simpler readings (Declared)

- **Existential first-cut** (`∃x:C`): gates immediately and matches the closed-class
  "demonstrative ≈ existential" first cut, but commits to ∃ where core-en stays neutral and most
  science bare-plurals are *generic*, not existential — it would silently mis-encode the logical form.
- **Generic/kind operator**: closer to the truth, but bakes in a generic quantifier with no warrant and
  needs a generic-operator semantics we don't have. The deferred hole **subsumes** both: the resolver
  may bind `Q` to ∃, a generic operator, or all-observed-cases — *whichever the cited reference
  supports* — instead of the grammar pre-deciding.

## 5. Implementation slices

1. **Carrier — the hole kind (DONE, slice 1).** `HoleKind::Quantification` added; `classify_felicitous`
   generalized from "every hole is `Entity`/`EntityRef`" to **per-hole `(type, kind)`** — it now
   discovers `$anaphor$` (Entity) and `$quant$` (`Π(A:Set).(A→Prop)→Prop`) holes per span and types
   each in `gamma` by its own type. A quantification hole classifies into the **open** forest and is an
   **output** obligation (survives), unlike `EntityRef`. Pronoun/possessive open-parse tests unchanged.
2. **Parser — the shift (DONE, slice 2).** A unary `cat_n(C, pl) → cat_np(C, pl)` rule
   (`LexicalIndex::bare_plural_nps`, seeded in `lookup_span` alongside `kind_subject`), reusing the
   loaded plural-existential determiner `these` (subject + object cats; dispatched by `fwd`/`bwd` body
   head) with the determiner sem replaced by the **η-expanded deferred** sem (§2). Gated on `pl`. The
   `$quanthole$` sentinel is freshened per span (`freshen_quant`) like `freshen_anaphor`.
   **Verified:** small-lexicon TDD (`bare_plural_np_is_a_deferred_quantifier_argument` — subject /
   object / both parse as **open**; bare singular does not shift), the de-risk probe, all 82 determiner
   + full kernel tests green; and **full lexicon** `genes affect cells` → **open×12, 0.0 s** (was no
   parse), no OOM.
3. **Output/institution — the discharge (TODO, Phase-2).** Emit each quantification hole as a typed
   grounding obligation on the warranted proposition; the encoding institution discharges it by binding
   `Q` and committing a `Citation` to a real `Reference` (D43 retrieval), upgrading the grade.
   (Phase-2-shaped, per the output contract §5; slices 1–2 already make bare plurals parse.)

## 6. Open questions (Declared)

- **Higher-order hole typing.** RESOLVED for the **gate**: the de-risk probe + the working feature show
  the kernel binds and type-checks a quantifier-typed neutral, **provided** the scope is η-expanded
  (§2) — a rigid predicate is rejected for want of contravariant arrow subtyping. Still open for
  `resolve_open`: binding `Q` to a concrete quantifier and re-gating is the Phase-2 discharge path.
- **Default reading when no citation is available.** A bare plural with no discoverable warrant: hold
  the obligation open (fail-closed, the reasoning protocol), or fall back to an explicitly-graded ∃?
  Lean toward *open* (never silently quantify).
- **Mass nouns.** core-en's `pl-or-mass` — add a `mass` number feature when mass terms enter scope.
- **`genes are large` (predicate over a bare plural).** Confirm the subject shift feeds the copula
  path, not only transitive verbs.
