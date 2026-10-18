# D63 — Coordinated modifier category (`cat_mod`): closing the heterogeneous-modifier over-generation

**Status:** design note + root-cause record. No code landed (a naive fix was prototyped, measured to
over-reach, and reverted — §7). The proper fix (a coordinable modifier category) is specified in §4;
the one open decision is §6.

**One line:** a pre-nominal modifier has no standalone category, so a bare noun cannot coordinate with
an adjective as a modifier; the grammar shoves it through a raised-form path instead, producing a
malformed reading. The fix is a `cat_mod` category that is *coordinable but never carries an abstract
head type `C`* — preserving the design's concrete-`Σ` invariant while letting modifiers meet each other
before they meet the head noun.

This is the residual **driver (1b)** from the structural-ambiguity analysis (the sibling
[nominal-modification normal form](d63-nominal-modification-normal-form.md) and the (1a) canonical
modifier-`And` fix, `experiments/parsing/baseline.json` history). (1a) killed the modifier-`And`
*commutativity* over-generation; (1b) is the coordination case, and it is a different, deeper defect.

## 1. The defect (Derived — controlled probes via `EIGENIUS_TRACE_SKELETONS`)

`trace_one_sentence` with senses erased to structural skeletons. It is **heterogeneity**, not list length:

| probe | modifiers | readings / structural skeletons |
| --- | --- | --- |
| "Ovarian cancers are common." | 1 adjective | 1 / 1 (encoded) |
| "Gastric, endometrial and ovarian cancers are common." | 3 adjectives (homogeneous) | 4 / 2 — clean |
| "Colon, gastric, endometrial and ovarian cancers are common." | noun + 3 adj | 40 / 12 |
| "Colon, breast and ovarian cancers are common." | 2 nouns + adj (heterogeneous) | 105 / 6, sense× 17.5 |

A homogeneous adjective list folds cleanly (`fold_conn` is already left-canonical); the moment a
**noun**-modifier ("colon"/"breast", which has no adjective category) joins the coordination it blows
up. The malformed restrictor on the head noun embeds an **object-GQ used as a modifier**, e.g.

```text
Σ__cmp_x:C1140680. And(compound_kind(__cmp_x, C1515981),
                       λconj0. And(λTV. λsubj. TV(kind_of(colon), subj)(conj0), ...)(__cmp_x))
```

`λTV. λsubj. TV(kind_of(colon), subj)` is "colon" **object-type-raised** — a functor waiting for a
transitive verb — appearing inside a noun's restrictor, which is nonsense.

## 2. Root cause — two layers

### 2a. Immediate mechanism: provenance laundering (Derived — `EIGENIUS_TRACE_MISMATCH`)

`refine_attrib` already guards against exactly this: `Guard::NotProv(Left, KindRaised)`
([combinators.rs](../../kernel/src/dcg/rules/combinators.rs) `refine_rules`) refuses a bare noun's
`KindRaised` raised form as a pre-nominal modifier. But **coordination erases the provenance**:

- `build_coordinate` ([registry.rs](../../kernel/src/dcg/rules/registry.rs)) and
  `apply_coord_complete` (the `CoordComplete` unary shift) both build results with `Item::with_cost`,
  which hard-codes `prov = Combinator::Other`.

Witnessed: `COORD-LAUNDER: l.prov=KindRaised r.prov=KindRaised -> result prov=Other`, operands being
bare-kind-raised nouns (`cat_np(n05535869…)` = "colon"). So a `KindRaised` form the guard would refuse
slips through **once coordinated** — its stamp now says `Other`.

### 2b. The real cause: no coordinable modifier category

Un-laundering the provenance is not enough (§7 proves it over-reaches). The provenance leak only
*matters* because pre-nominal modifiers have **three incompatible categories** and no shared one:

| modifier | category | rule |
| --- | --- | --- |
| adjective | `S[adj]\NP` | `refine_attrib` |
| named entity | `cat_np` | `refine_named_compound` |
| N-N compound | `cat_n` | `refine_kind_compound` |

Coordination coordinates *same-category* constituents. A noun-modifier and an adjective-modifier share
no category, so the coordinated-noun-modifier reading has **no clean derivation** — the parser reaches
it only via the raised-form path, which is the malformed one. Blocking that path without supplying a
correct one strands the sentence (§7).

## 3. Why the grammar has no modifier category — a deliberate choice (Derived — D63 §3b)

This is a **dependent** categorial grammar: semantics are terms of EigenTT, and a modified noun is a
dependent pair `Σx:C. restr(x)` indexed by the head noun's **concrete** ontology class `C`
([d63 §3b](../design/d63-dcg-engine-english-grammar.md), lines ~981–1009).

The attributive rule builds `cat_n(Σx:C. adj(x))` *"over the concrete `C` at parse time — so `adj(x)`
type-checks at `x:C` directly (sidestepping the bounded-quantification gap entirely; no abstract `C`)."*

A textbook CCG `N/N` modifier is exactly what *introduces* an abstract `C` (a standalone functor
polymorphic over whatever noun it meets), and that hits two **deliberate** kernel limitations:

1. **No coercive Σ-subtyping** — the kernel has no `Σx:C.R ≤ C` coercion (following Lean / `nanoda_lib`:
   coercions live in the engine/elaborator, never the trusted kernel; the `Fst`-projection is inserted
   by the engine).
2. **No clean bounded quantification** over an abstract `C`.

The binary type-changing rule avoids both by holding the concrete head noun. The doc records the
alternative was weighed: *"`lightblue`'s DTS underspecification-`@` model was the heavier alternative,
not adopted."*

So: the deviation from textbook CCG buys a minimal kernel; the **bill is coordination** — a modifier
never exists as a standalone constituent, and coordination needs one.

## 4. The design: a `cat_mod` category (Declared — hypothesis, not yet built)

The sharpened spec: **give modifiers a standalone, coordinable category *without* reintroducing an
abstract `C`.** Threadable because the head-type dependency can stay in the *sem* as an un-type-checked
lambda, with the `Σ` still built over the concrete `C` at application.

### Pieces

1. **Category ctor `cat_mod`** — nullary `lexicon:Cat` constructor, "a pre-nominal modifier awaiting a
   head noun." Carries **no `C`**; its sem is a restrictor term `λx. restr(x)` (a syntactic lambda, not
   a functor with a denotation). *(Confirm whether `Cat` ctors are ontology-declared — the
   `every_connective_… is declared in the ontology` test suggests yes → a bootstrap edit → reseed.)*

2. **Lift shifts** (into `unary_shifts()`), the restrictor-halves of today's `refine_*` builders,
   pulled out to stand alone:

   | lift | from | `cat_mod` sem |
   | --- | --- | --- |
   | adjective | `S[adj]\NP` | `λx. adj(x)` (identity on the predicative sem) |
   | attributive noun | `cat_n(M,_)` | `λx. compound_kind(x, M)` |
   | named entity | `cat_np(M,_)` | `λx. compound(x, M)` |

   Per-item unary shifts (like type-raise / bare-NP) → packable, covered by the differential oracle.

3. **Application rule** `cat_mod ⊕ cat_n → cat_n`: `cat_mod(restr) + cat_n(C, num) →
   cat_n(Σx:C. And(P, restr(x)), num)`, via the existing `refine_conjoin`/`conjoin_canonical` — over
   the **concrete `C`**, number flowing through, left-branching NF guard intact. **Subsumes** today's
   three binary refine rules. The `Σ` is built exactly where it is today, so "no abstract `C`" holds.

4. **`cat_mod` coordination** (the one design decision — §6). Must **not** use `coordinate_prop`
   (it calls `denote_cat`/`prop_ending` and η-expands — the abstract-`C` machinery we are avoiding); a
   dedicated term-level fold of the restrictor lambdas.

5. **Retire `NotProv(KindRaised)`** — with `cat_mod`, an object-GQ has category *object-GQ*, not
   `cat_mod`, so it cannot type-match the modifier slot. The guard becomes dead code; deleting it is the
   proof the bad behaviour was *eliminated*, not guarded.

### Structural decision: replace, don't add

Keeping `refine_attrib` *and* adding a parallel `cat_mod` lift doubles the derivations of every simple
"red ball." So **replace** the three direct refine rules with lift-shifts + the one `cat_mod`
application:
- `red ball` → `red` lifts, applies — one path;
- `colon and ovarian cancers` → both lift, coordinate as `cat_mod`, apply — the path that does not
  exist today.

Cost: one extra unary node per modifier, but all `cat_mod` items pack to one signature → bounded.

## 5. Invariants & constraints

- **Well-typedness (core):** a `cat_mod` sem is a restrictor `λx:C. Prop` — never a GQ/object-GQ/clause.
  This is what laundering violates; making it a *type* property retires the sem-blind guard.
- **No abstract `C`:** the restrictor is only ever type-checked at a concrete `C`, at application — the
  invariant the whole current design exists to protect (§3).
- **Number preservation:** application keeps the head's `sg/pl/mass` (already in `refine_conjoin`).
- **Single canonical bracketing:** stacked + coordinated modifiers collapse to one flat, canonically
  ordered `Σ` (integrate with (1a)'s `conjoin_canonical` + the D63 §8.13 left-branching NF).
- **Differential oracle:** packed ≡ unpacked — holds if lift/apply/coordination live in the shared
  `apply`/registry rules.
- **Felicity:** the refined noun type-checks.

## 6. Open decision — coordination semantics (union vs. also-intersective)

Stacked and coordinated modifiers differ:

- **Stacked / juxtaposed** ("large red ball") → **intersective**: `Σx:C. And(large x, red x)` — already
  what `conjoin_canonical` builds.
- **Explicitly coordinated** ("colon and ovarian cancers") → **union**: the plural ranges over
  {colon cancers} ∪ {ovarian cancers}; an individual is colon *or* ovarian, never both. Restrictor is
  `λx. Or(r1 x, r2 x)` ("and" = individual-level disjunction).

But "and" on modifiers is genuinely ambiguous — "a tall and handsome man" *is* intersective. So the
decision: does `cat_mod` coordination yield **union only** (correct for the WRN classifier lists), or
**both** union and intersective (a real but multiplicity-costly ambiguity)? Recommendation: **union
only** for the classifier case first, measure, and add the intersective reading only if a real sentence
needs it.

## 7. What was tried and reverted (Derived — measured, fail-closed)

Naive fix: un-launder the provenance — propagate `KindRaised` through `build_coordinate` and
`apply_coord_complete`. Tests + differential oracle stayed green. Effect:

- **Object position helped:** "MSI occurs in colon, gastric, endometrial and ovarian cancers"
  **82 → 30** readings (**19 → 5** skeletons).
- **Predicate position regressed:** "These groups are MSI lines, microsatellite-stable lines, …"
  **65 → 128** (+63) — blocking the raised-form path removed the *only* coordinated-modifier route,
  forcing a widen-explosion.
- Net cap-only −26 (2347 → 2321), grammar-gap 0 — but the +63 is a **new** over-generation on a real
  WRN sentence.

**Reverted.** The lesson is the point of this note: blocking without supplying the correct path is a
guard that over-reaches (the project posture forbids this). The `cat_mod` category supplies the correct
path, so the block is no longer needed.

## 8. Validation criteria (how we will know it is the right shape)

1. **oracle:** packed ≡ unpacked holds;
2. **guard removed** (not bypassed) — `NotProv(KindRaised)` deletable;
3. **cap-only per-unit:** the +63 predicate unit does **not** regress **and** the −52 object win holds;
4. **no doubling:** non-coordinated modifiers ("primary cell line", "cancer cell") stay flat, single
   structural skeleton.

Pieces 1–3 and 5 of §4 are mechanical restructurings of existing code (high confidence); §4.4
(coordination semantics) and the replace-not-add refactor carry the risk. The whole design is a
hypothesis until the four checks pass.

## 9. Update 2026-07-19 — M3 shipped, M2 failed, RNR reframing

### 9a. Status

M1 + M3 built and committed (`e7c1b24`), parser-only, no reseed. §4's `cat_mod` is built and the
replace-not-add refactor landed (`refine_attrib` + its `S[adj]\NP + cat_n` rule + `Guard::NotProv`
deleted; `mod_apply` is the sole attributive path; shift order `ModLift`-before-`CoordComplete` blocks
the coordinate-then-lift `And` leak). §8 checks 1/2/4 pass; check 3's +63 predicate regression was the
**naive** fix of §7, not `cat_mod` — it does not recur. §6 is **resolved: union only**.

### 9b. §6 resolved — the pivot is grammatical, not semantic (Derived)

The union-vs-intersective choice is not a per-list judgement; the **surface form** fixes it:
attributive comma-coordination ("X, Y and Z Ns") → union `Or`; predicative ("N is X and Y", "N that
is X and Y") → intersective `And`; bare stacking ("X Y N") → intersective `And`. Witnessed:
"Ovarian cancers are common and frequent" → `And(common, frequent)` (predicative, correct);
"Gastric, endometrial and ovarian cancers" was `And` (wrong), now `Or`. The **category is the pivot** —
`cat_mod` (attributive, lifted) folds `Or`; `S[adj]\NP` (predicative) keeps `coordinate_prop`'s `And`.
So union-only is what the attributive category *means*, not a scoped simplification. Measured: cap-only
2347→2321 (−26); reranked drift-free encoded 10→12, total 1027→1124 (a `SENSE_CAP=2` interaction, not
over-generation — the sense-independent signals are cap-only −26 and skeleton 19→5 on the object unit).
See `experiments/parsing/baseline.json` history[0].

### 9c. M2 (noun/NE modifiers join the union) — attempted, EXPLODED, reverted (Derived)

In "colon, gastric, endometrial and ovarian cancers", `colon`/`gastric` resolve to UMLS/WordNet **nouns**,
not adjectives, so they never lift to `cat_mod` and stay stacked (`And`) outside the `Or`. Attempt (no
global lift; `coordinate_mod` converts a `cat_n`/`cat_np` conjunct to a restrictor `compound_kind(x, N)` /
`compound(x, N)` inline, `kind_compound` nesting untouched): `colon` did join the union, but the unit
**exploded 30→64 readings / 5→32 skeletons**. Root cause: a noun gains a *coordinate* mode **on top of**
its `kind_compound` (stacking) + `cat_np`-group modes; in a mixed comma-list each noun independently
coordinates *or* stacks. Same additive-ambiguity trap M3 avoided by *deleting* `refine_attrib` — here
there is nothing clean to delete (real stacked compounds need `kind_compound`). Reverted; scratch
`m2-attempt.patch`.

### 9d. The reframing — coordinated modifiers are RIGHT-NODE RAISING over lexicalized compounds (Derived)

Each distributed "X cancer" traced as "X cancer is common":

| compound | own entry? | resolves to |
| --- | --- | --- |
| colon cancer | yes | WordNet `n14247239` |
| gastric cancer | yes | UMLS `C0024623` (+`C0699791`) |
| endometrial cancer | yes | WordNet `n14247458` |
| colorectal cancer | yes | UMLS `C0009402` |
| insertion mutation | yes | UMLS `C1512796` |
| deletion mutation | yes | UMLS `C1511760` |
| ovarian cancer | **no** | composes; explodes 140/20 |

So the faithful reading of "colon, gastric, endometrial and ovarian cancers" is **not** a generic cancer
with a disjunctive modifier — it is a **union of lexicalized kind concepts**
`Or(n14247239, C0024623, n14247458, ⟦ovarian cancer⟧)`. The current grammar cannot reach it: a multiword
lexeme needs **adjacency**, and in a shared-head coordination the head appears once at the end, so the
pre-modifiers are separated from `cancers` — the lexicalized concepts are **unreachable**, and `cat_mod`/`Or`
(correct as far as it goes) unions the right *modifiers* while throwing away the right *kinds*.

The construction is **right-node raising** (head-sharing coordination): distribute the shared head onto each
conjunct, re-run multiword lookup on each "X cancer", union the results. This (i) reaches the lexicalized
concepts; (ii) **dissolves the noun/adjective asymmetry M2 chased** — each modifier compounds with the head
regardless of POS, so head-distribution, not modifier-lifting, is the real operation; (iii) gives partial
lexicalization a clean home (the three that lexicalize become atomic, only "ovarian cancer" composes).

**Page prevalence** (Derived — grep + probes): three shared-head coordinations are RNR-defeated —
"colon, gastric, endometrial and ovarian cancers" (para 2), "colorectal, endometrial, gastric and ovarian
cancers" (para 4, a live measurement unit), "insertion or deletion mutations" (para 2, both compounds
lexicalized — the cleanest case). "MSI lines, microsatellite-stable lines and indeterminate lines" (para 3)
is head-**repeated**, not RNR. Lexicalized compounds are pervasive (7/8 probed → a single CUI: also cell
cycle arrest `C1155873`, Lynch syndrome `C1333990`, homologous recombination `C0599773`, immune checkpoint
blockade `C5392067`).

### 9e. Failure mode 2 (composition not suppressed) — investigated, NOT a page lever (Derived, corrected)

An initial claim that standalone compounds explode and inflate the page was **wrong**. The page uses
**plural, lexicalized** forms: "cell lines" → `C0007600`, one reading; "These MSI cell lines were distinct"
→ one reading (an encoded unit). Residual multiplicity in the other "cell lines" units ("three groups of
cell lines" 26, "screened cell lines with a CRISPR library" 32) is PP-attachment / verb-argument, not
cell-lines. The singular "cell line" (136/24) and the standalone "ovarian cancer" (140/20) are **coverage
gaps for surfaces the page never uses bare**. The multiword-preference works on the page; log, do not fix.

### 9f. Direction (Declared)

The M2-superseding move is **right-node raising**: head-distribution + per-conjunct multiword re-lookup,
with `cat_mod`/`Or` as the fallback where the distributed compound does not lexicalize. Coverage gaps to
log (not fix now): bare singular "cell line"; the "ovarian cancer" surface (concept exists as "ovarian
carcinoma" `C0919267`). `cancer`/`line` junk senses (zodiac, tropic) are down-rank candidates, not blockers.

## References

- D63 §3b — the concrete-`Σ`, no-abstract-`C`, no-kernel-coercion rationale:
  [d63-dcg-engine-english-grammar.md](../design/d63-dcg-engine-english-grammar.md) (~981–1009).
- Nominal-modification normal form (the `Σx:C. R` model, left-branching NF):
  [d63-nominal-modification-normal-form.md](d63-nominal-modification-normal-form.md).
- Eisner normal form (provenance-to-constrain-derivations, of which `KindRaised` is an Eigenius
  extension): `references/publications/Eisner-…Normal Form Parsing.pdf`.
- Code: `refine_rules`/`refine_conjoin`/`conjoin_canonical`
  ([combinators.rs](../../kernel/src/dcg/rules/combinators.rs)); `build_coordinate`/
  `apply_coord_complete` ([registry.rs](../../kernel/src/dcg/rules/registry.rs)); `coordinate_prop`
  ([constructions.rs](../../kernel/src/dcg/rules/constructions.rs)).
- Instruments: `EIGENIUS_TRACE_FOREST` (derivation forest), `EIGENIUS_TRACE_SKELETONS` (structural
  skeletons) on `trace_one_sentence` (`crates/eigenius-wordnet/tests/db_backed_encoding.rs`).
