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
