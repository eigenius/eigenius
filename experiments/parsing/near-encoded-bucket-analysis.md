# Near-encoded ambiguity: what the 2–8-reading units actually contain

**Question.** For the WRN first page (CNL v3), the units closest to a single reading — the 2–8
reading bucket — what distinguishes their competing readings: structure, or sense?

**Answer up front.** Structure dominates even here. Of the 15 units in the bucket, **7 are purely
structural** (`sense× = 1.0`), 2 purely sense, 6 mixed — so **13 of 15 carry a structural
component**. The sense component, where present, is part *catchable* cross-lexicon leftovers (a few
specific CUIs the aligner missed) and part *irreducible* polysemy.

---

## Provenance

| | |
|---|---|
| snapshot | `wordnet-umls-aligned-v3-2026-07-12` (38,389-merge alignment) |
| source run | `experiments/parsing/results/2026-07-12-2002-…-first-page-cnl-v3-reranked` |
| method | `dive_near_encoded` over `near-encoded-bucket-page.txt` (the 15 bucket sentences, one paragraph), **replaying that run's `ranks.json`** so the reading counts match the measured ones |
| unblocked by | the fallible-readback fix (`kernel/src/nbe/readback.rs`) — before it, the dive crashed (GH#104 readback panic) on the coordination units before reaching most of the page |

Reproduce:

```bash
EIGENIUS_DB_SNAPSHOT=<v3 snapshot> \
EIGENIUS_SENSE_RANKS=<source run>/ranks.json \
EIGENIUS_WRN_PAGE=experiments/parsing/near-encoded-bucket-page.txt \
  cargo test --release -p eigenius-wordnet --features use-llm --test db_backed_encoding \
  dive_near_encoded -- --ignored --nocapture
```

`reads` = felicitous full-span readings; `skels` = distinct **structural skeletons** (every sense
IRI erased to `§`); `sense× = reads / skels` (1.0 ⇒ purely structural).

---

## The 15 units

| # | unit | reads / skels / sense× | class | what the readings are |
|---|---|---|---|---|
| 1 | Each event alone does not lead to cell death | 4 / 2 / 2.0 | mixed | `event` sense (n00029378⇄n13943400) + 2 structural |
| 2 | Scientists can exploit synthetic lethality for cancer therapeutics | 6 / 2 / 3.0 | mixed | `therapeutics` senses + compound `cancer therapeutics` vs PP `for cancer` |
| 3 | PARP-1 inhibitors are successful in cancers with deficiencies in homologous recombination | 2 / 2 / 1.0 | structural | PP attachment |
| 4 | This success highlights the potential of this approach | 2 / 1 / 2.0 | sense | `potential` / `approach` polysemy |
| 5 | We found that WRN was selectively essential in MSI models | 5 / 5 / 1.0 | structural | complement + adverb + PP attachment |
| 6 | MSI cancer models required the helicase activity of WRN | 8 / 8 / 1.0 | structural | compound `MSI cancer models` + `activity of WRN` |
| 7 | Defects in DNA mismatch repair promote a hypermutable state | 8 / 4 / 2.0 | mixed | `state` sense (n14464005⇄n05162642) + PP/compound |
| 8 | MSI contributes to several cancers | 2 / 2 / 1.0 | structural | PP / verb attachment |
| 9 | MSI can arise from Lynch syndrome | 6 / 6 / 1.0 | structural | modal scope (`can` → Possible vs And) + `arise` verb-sense as distinct skeleton |
| 10 | Germline mutations in the MMR genes MSH2, MSH6, PMS2 or MLH1 cause Lynch syndrome | 2 / 1 / 2.0 | sense | `gene` cross-lexicon (n05436752⇄**C5849123**) |
| 11 | Thus, MSI tumours need novel therapies | 4 / 2 / 2.0 | mixed | `therapies` cross-lexicon (C0039798⇄n00661091) + structural |
| 12 | We analysed these data sets for genes that are selectively essential in cancer cells with MSI | 8 / 4 / 2.0 | mixed | `gene` cross-lexicon (n05436752⇄**C5849123**) + relative-clause attachment |
| 13 | WRN encodes a RecQ DNA helicase | 3 / 3 / 1.0 | structural | compound bracketing |
| 14 | These MSI cell lines were distinct | 3 / 3 / 1.0 | structural | compound bracketing |
| 15 | These lines possess events that are predictive of MMR deficiency | 8 / 2 / 4.0 | sense-heavy | `events` (C1705644⇄C1879775) + `lines` (n08430568⇄C0205132) |

---

## The two axes

**Structural** (13 of 15 units) — a small fixed set of phenomena, none reachable by lexicon work:

- **Compound-noun bracketing** — `MSI cancer models` (8 skeletons alone), `RecQ DNA helicase`,
  `MSI cell lines`, `cancer therapeutics`, `DNA mismatch repair`.
- **PP attachment** — `in cancers with deficiencies in homologous recombination`, `activity of WRN`,
  `in MSI models`.
- **Modal scope** — `can arise` reads as both `Possible(…)` and a conjunction.
- **Relative-clause / complement attachment** — `genes that are…`, `found that…`.

**Sense** (8 of 15 units have a sense component) — three kinds:

1. **Unmerged cross-lexicon duplicates** — the *same concept* the aligner did not merge:
   `gene` n05436752⇄**C5849123** (units 10 **and** 12), `therapies` C0039798⇄n00661091 (11),
   `therapeutics` n04074482⇄C0087111 (2). Concretely fixable by alignment — `C5849123` is a
   different gene CUI than the `C0017337` that *was* merged, and it alone costs two units. See
   `experiments/lexicon-align/`.
2. **Genuine WordNet polysemy** — `event`, `state`, `potential`. Distinct senses; alignment cannot
   touch these.
3. **UMLS-internal near-synonyms** — `events` C1705644⇄C1879775.

---

## Bottom line

At fine grain this matches the aggregate result: alignment removed the cross-lexicon duplicates it
could reach; what remains in the near-encoded units is **dominated by structure** (compound
bracketing + PP/clause attachment), with a minority sense residue that is part a handful of
still-catchable CUIs (`C5849123`, the therapy pairs) and part irreducible polysemy. The lever for
this bucket is structural disambiguation, not more lexicon merging.

---

## Deep dive — noun-compound bracketing (`MSI cancer models …`)

Dumped with `EIGENIUS_DIVE_SKELETONS=1` (env-gated skeleton dump in `dive_near_encoded`), the 8
readings of **`MSI cancer models required the helicase activity of WRN`** (8 readings / 8 skeletons /
`sense× = 1.0` — purely structural) factor into **two independent axes** that multiply, `2 × 4 = 8`:

| axis | what varies | skeleton evidence |
|---|---|---|
| **A. subject NP** (×2) | an extra intersective conjunct present or not | skels 0–3 `And(prep_of(G#1, kind_of(§)), λG#2. G#1(kind_of(§), G#2))` vs skels 4–7 `prep_of(G#1, kind_of(§))` |
| **B. object NP** (×4) | `compound_kind` vs `And`, and flat vs nested | `compound_kind(G#2, §)` (flat) · `compound_kind(G#2, compound_kind(G#3, §))` (nested) · `And(compound_kind(…), λG#3.…)` · `And(λG#3.…, λG#3.…)` |

The two grammar choices behind this:

1. **left- vs right-branching** for 3+ nouns — `compound_kind(x, compound_kind(y, z))` (nested) vs
   flat. Eigenius **already** has a partial fix: the left-branching normal form `is_compound_refined`
   (D63 §8.13, `kernel/src/dcg/parser.rs:870`) forbids a compound-refined noun as a compound
   **head**, collapsing head-side spurious brackets. **Gap:** it does not forbid a compound-refined
   **modifier**, so modifier-side nesting survives (the flat skel 2/6 vs nested skel 3/7). Extending
   the NF to the modifier side is a low-risk kill of axis-A/B nesting.
2. **`compound_kind` vs intersective `And`** — Eigenius splits nominal modification into
   `KindCompound` (`[cat_n][cat_n] → compound_kind`, `parser.rs:409`) and the attributive/conjoining
   path that builds a flat-Σ `And`. Where both are licensed for one span, both semantics survive.
   **Traced (2026-07-12):** the `And` is licensed by a **mass-number modifier**, not a lexical
   adjective or a named individual. Controlled isolation:
   - `cell lines` (2 count nouns) → no `And`.
   - `MSI cell lines` → the `And` appears; `MSI` is the **only** word here with a `mass` variant
     (`cat_n(umlscui:C0920269, mass)` alongside `num_any`).
   - `tumour cell lines` (all count nouns, `tumour` count-only) → **no `And`**, only compound.

   **Nailed by exact sem identity.** The `And`'s second conjunct is
   `λG#2. G#1(kind_of(C0920269), G#2)` — literally the object-raised sem from `kind_raised_nps`
   (`kernel/src/dcg/lookup.rs:953`, `"bwd"` branch: `λTV. λsubj. TV(kind, subj)`) with `TV = G#1`,
   `kind = kind_of(MSI)`. So the `And` is **not** a "mass → intersective modifier" rule; it is the
   **bare-mass NP shift**: MSI's `mass` variant is kind-raised to a bare *argument* NP by
   `bare_mass_nps` (`lookup.rs:1006`), and that raised NP is then consumed as a **pre-nominal
   modifier**, conjoined via `And` alongside the genuine `compound_kind`.

   Two facts close it: (i) `bare_nominal_shifts` runs on **composed cells too** (`lookup.rs:1022`),
   so `cell lines` is itself a bare NP — which is why the spurious reading needs a *compound* head:
   `MSI cells` (simple head) is ENCODED, no `And`; (ii) `tumour` has no `mass` variant → no shift →
   no `And`. So a mass/plural noun that legitimately kind-shifts for *argument* position
   ("MSI is a state") is being allowed to serve as a *pre-nominal modifier* — the over-generation.

### Comparison with core-en (OpenCCG reference, `references/openccg/grammars/core-en`)

- **core-en has no productive noun-noun compound rule.** A pre-nominal modifier is an attributive
  **adjective** only — category `n/n` (`$adj`, `adj.xsl:20`) with one `HasProp` semantics. The
  type-changing rules are `rrel` / `tpc` / `bnp` / `card` / `card-h` (`unary-rules.xsl`); **none**
  turns a noun into a modifier, and there is no `n/n` noun family in `dict/np/lexicon`. So core-en
  would not generate `MSI cancer models` as a productive compound at all — it avoids the ambiguity by
  not having the construction (too lossy for compound-dense biomedical text).
- **Eigenius added productive compounds** (`KindCompound`, D63 §8.13) to parse exactly these spans,
  and **split** modification into `compound_kind` vs attributive `And` where core-en keeps a single
  `n/n HasProp`. The noun-bracketing ambiguity is the price of that extension.

### Fix directions (to be weighed)

- **Canonicalize bracketing** — extend the left-branching NF (`is_compound_refined`) to the modifier
  side. Kills the flat/nested split (axis-B nesting, part of A). Low risk. General to all 3+-noun
  compounds (`tumour cell lines` shows the same flat/nested leak).
- **Stop a kind-raised bare NP from serving as a pre-nominal modifier** — the `And` is a
  *bare-mass/plural NP shift* (`kind_raised_nps`, `lookup.rs:953`) whose raised NP, meant for
  *argument* slots, is being consumed as a noun modifier. Gate the combination so a kind-raised bare
  NP fills only argument positions, not pre-nominal-modifier ones; the genuine `compound_kind`
  survives. Kills axis-2 for MSI and every other mass/plural modifier, and leaves argument-position
  bare NPs ("MSI is a state") untouched — so `grammar-gap` stays 0.

### Fix — RESOLVED (`2026-07-16`), and it needed BOTH directions, in order

The two directions above are **not alternatives** — direction 2 alone *gaps* grammatical sentences,
because the over-generation is **load-bearing**. Witnessed (dumped sems on the post-span-integrity
`v3-2026-07-15` snapshot):

- The spurious `And` is `refine_attrib` consuming a bare mass/plural noun's **predicative `S[adj]\NP`**
  kind-raise (sem `λTV.λsubj. TV(kind, subj)`, from `kind_raised_nps`'s `bwd` branch — the `a`/`these`
  determiner set carries the predicative body alongside the object-GQ, both `bwd`-headed) as a
  pre-nominal modifier.
- Gating that consumption (or deleting the predicative form) correctly kills the `And` — and **gaps**
  `MSI cell lines from these four lineages were distinct` and #8 (unit 54): their ONLY parses *were*
  the over-generation. Cause: **`refine_attrib` is the only modifier rule that FLATTENS** a further
  modifier onto a compound-refined noun (`Σx:Base. And(P(x), q(x))`); `refine_pp_mod` /
  `refine_kind_compound` / `refine_named_compound` **nest** (`Σy:(Σx:Base. P(x)). q(y)`, via
  `refined_noun`). So a compound + PP (`MSI cell lines` + `from …`) had **no** clean flat structure —
  only the over-generation's `And` supplied one.

**The fix (`kernel/src/dcg/rules/combinators.rs`, `item.rs`, `registry.rs`), two parts:**

1. **`refine_pp_mod` flattens** when its base `C` is already `Σx:Base. P(x)` → `Σx:Base. And(P(x),
   pp(x))`, mirroring `refine_attrib`. This *creates the clean compound+PP reading*
   (`kind_of(Σx:cell_lines. And(compound_kind(x, MSI), prep_from(x, lineages)))`).
2. **`refine_attrib` gated** with a `Guard::NotProv(Left, Combinator::KindRaised)` — `kind_raised_nps`
   now tags its outputs `KindRaised` (a new ENF-inert provenance), and the attributive rule refuses a
   `KindRaised` left. Removes the spurious modifier use; safe now that (1) supplies the clean reading.

**Verified:** object-raised `And` occurrences on the 2–8 bucket **16 → 0**; `MSI cancer models …`
**4 → 2** readings (1 structural skeleton, pure sense); **`grammar-gap` 0** on the full first page
(every sentence, incl. #8 and the bare-plural predications, parses cleanly); 1630 kernel lib tests
pass. Direction 1 (extend the NF / flatten the compound builders too) remains a **follow-on** for
3+-noun compounds not on this page — this fix flattened only `refine_pp_mod`, which the page needed.

---

## Deep dive — modal scope (`MSI can arise from Lynch syndrome`)

Unit 9 (6 readings / 6 skeletons / `sense× = 1.0`) carries two axes; one is genuine structure, one
is a verb *sense* split the metric miscounts (see the metric note below).

**Modal scope (structural).** `can` is a VP-to-VP modal, sem `poss_sem` (the `Possible` operator):

```
can : (S[fin]\NP)/(S[bse]\NP)
```

The skeletons split on where `Possible` scopes relative to the `from Lynch` adjunct:
- `Possible(And(arise, prep_from))` — `Possible` **high** (adjunct inside the modal)
- `And(Possible(arise), prep_from)` — `Possible` **low** (adjunct outside the modal)

**Nailed to the line.** The `from` VP-adjunct category (`ontologies/lexicon/closed-class.esl:966`,
shared by the whole VP-adjunct preposition family — lines 975/1002/1011/1020/1029) is

```
((S[fin_any]\NP)\(S[fin_any]\NP))/NP
```

with **`fin_any`** (polymorphic finiteness) on *both* the argument and result VP. So after taking its
object, `from Lynch` = `(S[fin_any]\NP)\(S[fin_any]\NP)` and `fin_any` unifies with either the base
VP `arise` (`S[bse]\NP`, *inside* the modal → `Possible` high) or the finite VP `can arise`
(`S[fin]\NP`, *outside* → `Possible` low). The `is_vp` combinability check (`parser.rs:455`) ignores
finiteness too, consistent with the lexical polymorphism. (The other axis — `arise` v00339738 vs
v02624263 — is a verb *sense* split, reranker territory.)

**Fix.** Not "pin the adjunct to `bse`" — a plain clause with no modal (`MSI arises from Lynch`) has
only a finite VP, so a `bse`-only adjunct would fail to attach → grammar gap. The ambiguity exists
*only* when a modal splits the VP into two levels, so the fix is an **Eisner-style normal form scoped
to modal contexts**: when a modal is present, admit only the adjunct-inside-the-base-VP derivation
(`Possible` high, the intended reading) and drop the outer one — exactly analogous to
`is_compound_refined`. The no-modal case is untouched, so `grammar-gap` stays 0.

---

## Metric note — `sense×` under-counts sense/lexical ambiguity

The dive's `sense×` (= readings / distinct skeletons, where a skeleton is the sem with senses erased)
**mislabels lexical-category ambiguity as structural.** `erase()` only collapses an `X:sense` IRI
*suffix*; a lexical-category clash produces different *functors and arguments*, which are not `:sense`
suffixes and so survive erasure as distinct skeletons. Witnessed:

- `several cancers` (unit 8): `compound_kind(…, C0443302)` vs `gt(deg_a00494409, std)` — `several`
  carries an attributive-adjective entry (`bwd(cat_s(dcl, adj), cat_np(…))` → `gt`) **and** its
  surface matches the UMLS noun `Several` (`cat_n(C0443302)` → `compound_kind`). A lexical clash,
  reported `sense× = 1.0`. (Full breakdown in the `gt`-vs-`compound_kind` deep dive below.)
- `homologous` (unit 3): the same shape — adjective (`gt`) vs a lexicalized concept (`compound_kind`).

**Fixed (2026-07-12).** `erase()` now runs a second pass that collapses any bare ≥4-digit sense id
(a CUI's `C0920269`, a WordNet offset `n05436752`, a synset number inside a predicate name
`v02624263_i` → `v§_i`), keeping the categorial part and the `G#N` structural vars. Re-running the
2–8 bucket:

| | old skeletons | new skeletons |
|---|---|---|
| total (14 units) | 44 | **40** |
| `MSI can arise from Lynch syndrome` | 6 | **3** (the `arise` verb-sense `v00339738`/`v02624263` collapses) |
| `We found that WRN was selectively essential…` | 5 | **4** |

So **4 of the 44 "structural" skeletons were verb-sense artifacts.** The remaining 40 are genuine.

**Honest limit:** `several`/`homologous` (units 8, 3) **stay** at 2 skeletons — the adjective-vs-noun
clash produces different sem *shapes* (`gt(deg_a§, std_a§)` vs `compound_kind(…, §)`), which no
sense-erasure collapses. They are *lexicon/reranker* work, not structural, so the metric cannot
reclassify them. Read `sense× = 1.0` as "no sense-of-one-word variation," and remember a
lexical-*category* clash still shows as two skeletons.

---

## Deep dive — `gt` vs `compound_kind` (gradable adjective vs noun compound)

`gt(deg_X(x), std_X)` is the **gradable-adjective positive form** (`kernel/src/dcg/category.rs:1437`
— degree function `deg_X`, contextual standard `std_X`; Chatzikyriakidis & Luo degree semantics). The
clash: **one surface matches both an adjective entry (→ `gt` via `Attrib`) and a noun entry (→
`compound_kind` via `KindCompound`)**, and both fire as pre-nominal modifiers. It is **two classes**
under one skeleton signature:

| unit | source of `compound_kind` | verdict | fix |
|---|---|---|---|
| `several cancers` (8) | `Several` = `cat_n(umlscui:C0443302)`, **TUI T081 Quantitative Concept** — a junk metadata concept seeded as a content noun (case-insensitive lookup lets lowercase `several` match capitalized `Several`) | **spurious** | **upstream**: don't seed T081 / qualifier / attribute TUIs as content nouns (the junk-entry class — same as `C0686904 "Patient need for"`); or the reranker kills it |
| `homologous recombination` (3) | `C0599773 "homologous recombination"`, **TUI T045 Genetic Function** — a real lexicalized concept | **both valid** (compositional `gt` vs lexicalized `kind_of(C0599773)`) | **multiword preference**: prefer the lexicalized concept when it fully covers the span; or reranker |

So the `gt`-vs-`compound_kind` clash is **lexical over-generation, not grammar structure** — junk
content entries plus a multiword-granularity choice. Neither is fixed by a normal form; both are
lexicon/reranker work. This is why the `sense×` metric cannot reclassify it (the sems differ in
shape) yet it is not "structural" in the actionable sense.

### Sizing the junk-TUI content-entry fix (measured, and modest)

Extracting every UMLS CUI that appears in the 2–8 bucket's readings (`EIGENIUS_DIVE_RAW=1`) and
looking up its semantic type: **6 of 32 distinct CUIs are junk metadata TUIs seeded as content
nouns** — `Several` (T081 Quantitative), `New`/`Successful` (T080 Qualitative), `Data Set` (T170
Intellectual Product), `Deficiency`/`therapeutic aspects` (T169 Functional). But only **2 of the 6
actually cause count-ambiguity** (they appear in *some* readings, competing with a correct entry):

| junk CUI | unit | appears in | if filtered (keeping the real entry) |
|---|---|---|---|
| `C0443302 Several` | 8 `MSI contributes to several cancers` | 1/2 readings | **2 → 1 (ENCODED)** — `several`'s `cat_measure` remains |
| `C0039798 therapeutic aspects` | 11 `Thus, MSI tumours need novel therapies` | 2/4 readings | **4 → 2** — WordNet `therapy` remains |

The other 4 (`New`, `Successful`, `Data Set`, `Deficiency`) appear in **all** readings of their unit —
they are a *wrong-denotation* quality problem (`novel` → "New", `successful` → a Qualitative Concept),
not a count-ambiguity source. **So the junk-TUI filter is not a broad count lever here** — it collapses
~1 unit to ENCODED and halves one more; its bigger value is correctness (6 words denoting a metadata
concept instead of their ordinary sense). It must keep a non-junk fallback entry, or filtering a
word's *only* entry causes OOV → grammar gap. (This downgrades the earlier "single highest-leverage
lexical fix" framing — measured, it is modest on count, broad on quality.)

**Related lexical junk, different mechanism.** The `genes ↔ C5849123` pair (units 10, 12), earlier
mislabeled a cross-lexicon *duplicate*, is a **spurious surface collision**: `C5849123` is a T033
Finding ("Gross Extranodal Extension") carrying a junk synonym atom **`gENE`**. Not a duplicate to
merge — a bad atom to suppress. A reminder that "cross-lexicon duplicate" and "spurious collision"
look identical in the skeleton and must be told apart by the concept, not the surface.

### Why the reranker's elimination doesn't stick — widen-on-failure

`C5849123` looks like a reranker miss, but it isn't. In all three `genes` sentences the recorded
ranking (`<run>/ranks.json`) is `order: [0, 1]` — the reranker **kept** WordNet `gene` and the real
UMLS `C0017337` and **omitted `C5849123`**. So the LLM eliminated it correctly, and the base-cap cut
(`lookup.rs`, "take no more than the ranker kept") drops it at the base cap.

It reappears because **the sentence widens.** Widen-on-failure (`lookup.rs:866`) is a *fail-open*
safety net: when a sentence can't parse at the base sense cap, the parser re-widens every word's
sense pool and **ignores the reranker's elimination** — "a wrong elimination therefore costs a slower
parse, never a grammar gap." So on any sentence hard enough to widen — precisely the ambiguity-heavy
near-encoded ones — *all* eliminated senses come back, `C5849123` included. The `n05436752 ⇄ C5849123`
axis in the parse is the fingerprint of a widen, not a reranker failure.

**Consequence for the fix — and it is NOT "delete the entry."** A lexical entry cannot be removed
in general: the concept is usually appropriate in *some* context (`Data Set` is a real noun; `several`
a real word). The `C5849123` cases split three ways:

1. **Bad atom** (`gENE → C5849123`): a miscased synonym atom wrong in *every* context. Suppress the
   **surface form**, not the concept — an atom-level data fix, not an entry deletion.
2. **Wrong category** (`Several` = T081 Quantitative, `Successful` = T080 Qualitative, seeded as
   `cat_n`): the concept is fine but a quantifier/adjective should not be a content noun. Fix the
   **import category**, don't delete.
3. **Genuinely context-dependent** (`Data Set` = T170; gradable adjectives; `C5849123` in a *widening*
   sentence): the reranker already makes the right contextual call — it *eliminated* `C5849123`. The
   lexicon is not the lever here. What defeats the reranker is the **widen**, so the fix is to remove
   whatever *forces* the widen (below), and let the contextual elimination stand.

So a blanket junk-TUI filter is itself too blunt (T170 covers real nouns); only (1) and (2) are
lexicon work, and both are surgical (atom / category), not deletion.

**Trace of the bypass — nailed: widen-on-failure.** Instrumenting the cap loop
(`parse_packed_at_cap`, `EIGENIUS_PARSE_DEBUG=1`) on `We analysed these data sets for genes that are
selectively essential…`:

```
cap=Some(2)  →  candidates=0     (base cap fails — no finite-clause parse)
cap=Some(4)  →  candidates=256   (widen fires, succeeds)
```

Base cap yields **zero** parses; `widen_packed` (`lookup.rs:1436`) escalates 2→4, and the cut only
applies when `Some(cap) == self.sense_cap` — so at cap 4 it is **skipped**, re-admitting every
eliminated sense. The whole 8-reading forest is a *cap-4* forest, which is why a reading using only
base-cap-subset senses coexists with the `C5849123` ones (all 8 are cap-4 readings — base cap never
succeeded). It is **not** the `if ranked > 0` guard: for `are`, `ranked = 1` (the closed `be` is
ranked), so `eff = min(2,1) = 1` and `be.v.02604760` **is** eliminated at base cap; same for
`genes`/`C5849123`. Both are eliminated at cap 2 and only return at cap 4.

The widen **trigger is `are`**: base cap seeds only the closed copula `be`, and `that are selectively
essential` has no parse on `be` alone at cap 2 (`genes` parses fine on `n05436752`, so it is not the
blocker — just collateral swept back in when the cap widens).

**So `C5849123` is a *symptom of the widen*, and the widen is a symptom of a base-cap parse gap** —
apparently the closed copula `be` not composing with a predicative "selectively essential" in a
relative clause. If that is a grammar gap, fixing it makes base cap parse, the reranker's elimination
**sticks**, and `C5849123` disappears **with no lexicon change at all**. The elimination *is* the
right contextual call; the load-bearing fix is to stop forcing the widen — either close the grammar
gap so the base cap parses, or (for the genuinely bad atom / wrong-category cases above) fix the
surface form or import category. "Remove the entry" is never the answer, because the entry is
appropriate in other contexts. **Open:** confirm the base-cap gap is `be` + predicative-adjective
relative clause (needs a reranked test sentence isolating it).

## Definite-as-existential negation-scope over-generation — FIXED (`2026-07-16`)

Analysing the 2–4-reading sentences, the one **grammar** (structural) over-generation among them was
`MSI cancer models did not require the exonuclease activity of WRN` — 2 readings, `sense× = 1.0`,
differing only in the position of `False`:

```
raw[0]: … require(…) → G#0 → G#0 → False      (¬∃ : negation wide)
raw[1]: … require(…) → False → G#0 → G#0      (∃¬ : negation narrow)
```

The `ΠG#0:Prop … → G#0 → G#0` chain is the CPS existential `∀C:Prop. (∀x:T. (TV(x,subj)→C)) → C` of
the **object determiner** `the`. Negation composes at two points in it, giving `¬∃`/`∃¬`.

**Root cause (witnessed).** Definite/demonstrative determiners reused `obj_exists_sem` /`exists_sem`
(a documented first-cut, closed-class.esl). For a genuine existential the two scopes are truly
distinct — confirmed: `HeLa did not require an activity` keeps both, correctly. For a **definite**
(unique reference) they collapse, so the second reading is spurious — confirmed: the split needs a
definite (`the`) *and* negation; `HeLa required the activity` (no neg) and `HeLa did not require
activity` (no `the`) are each a single reading.

**Fix — referential definite.** A definite is referential, not quantificational. New opaque axiom
`ontology:the : forall (A : Set) => A` (the ι operator — the presupposed unique referent of a
noun-type). Two sems: subject `λA.λV. V(the(A))`, object `λT.λTV.λsubj. TV(the(T), subj)`. The 12
definite entries (`the`, `the`-pl, `this`, `that`, `these`, `those`; subj+obj) point at them;
`a`/`an`/`some` and the cardinals keep the quantifier sems (their scope split is real).

**Why it collapses.** The category is unchanged (sem-only edit), so the categorial derivation set —
and thus **grammar-gap — is unchanged by construction**. Both scope derivations now assemble to the
identical `TV(the(T),subj) → False`; NbE normalises them equal; `felicity::subsume_duplicates` (the
definitional-equality dedup) drops the duplicate → 2 → 1.

**Status — DONE (`2026-07-16`).** Bootstrap typechecks (`the(A):A`, `TV(the(T),subj)` well-typed);
1631 kernel lib tests pass. The bootstrap edit invalidates the persisted chain, so the snapshot was
reseeded `--umls-all` + v3-aligned (2.8 GB, matching the baseline coverage — a first reseed at the
default WRN-TUI subset dropped ~2/3 of UMLS and produced spurious cap-only gaps + missing-lexemes;
not the fix). Witnessed on the faithful snapshot:
- cap-only: **grammar-gap 0, missing-lexeme 0**; the WRN sentence **4→2** (1 structural skeleton, the
  residual is a cross-lexicon `activity` sense dup, not scope).
- reranked (cnl-v3): the WRN sentence **AMBIG×2 → ENCODED**, so **encoded 6→7** — recovering the −1 the
  bare-mass run showed and matching the `6914d01` baseline (7); grammar-gap 0; the other 6 encodings
  intact.
- Existentials (`a`/`an`/`some`) and adjectival negation (`were not essential`) correctly untouched.

**Regression guards:** a CI-runnable lexicon-wiring test (`dcg::lexicon::referential_definite_tests`,
no snapshot — catches a definite→existential reversion or an existential→referential over-correction)
plus a snapshot-gated behavioural test (`definite_negation_collapses_referential` in
`crates/eigenius-wordnet/tests/db_backed_encoding.rs` — the definite is scopeless while the matched
existential keeps the `¬∃`/`∃¬` split).
