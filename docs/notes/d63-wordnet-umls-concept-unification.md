# D63 — Unifying common concepts between WordNet and UMLS

**Goal, and nothing else.** When WordNet and UMLS name the *same concept*, make the lexicon denote
**one** concept instead of two. Every word where they don't currently agree costs a factor of two in
readings, and half the sense cap.

**Status:** plan. The measurement harness this depends on is done and committed (`fa67033`).

---

## 1. The evidence this rests on (measured 2026-07-11, not asserted)

From `experiments/parsing/results/2026-07-11-1702-…/ranks.json` — the reranker's own recorded
decisions over the WRN page (407 content words ranked):

> **47% of ranked words spend BOTH `SENSE_CAP` slots on a cross-lexicon pair** — one UMLS sense and
> one WordNet sense of the same word.

And those pairs are routinely the *same concept*:

| word | UMLS gloss | WordNet gloss |
|---|---|---|
| **`state`** | "The way something is with respect to its main attributes." | "the way something is with respect to its main attributes" |
| `repair` | "The act of returning something to working order." | "the act of putting something in working order again" |
| `mismatch` | "A failure to correspond or match…" | "a bad or unsuitable match" |

`state` is **verbatim identical**.

**Why this costs readings.** The two entries are structurally identical and differ only in the class
IRI:

```
umlscui:e_C1442792_0   form="state"  cat=cat_n(umlscui:C1442792, num_any)  sem=umlscui:C1442792
wn:e_n00024720_0       form="state"  cat=cat_n(wn:n00024720,   num_any)    sem=wn:n00024720
```

The parser builds a reading for each. They are **not `Exp`-equal** (different IRIs), so
`subsume_duplicates` (D3) cannot collapse them — they survive as distinct readings and **multiply**.
That is the `sense×` axis: **median 5.5 per unit** (`results/2026-07-10-ambiguity-factoring`).

**Baseline to beat:** 62 units → `grammar-gap 0`, `missing-lexeme 0`, **ambiguous 60, encoded 1**.

---

## 2. The mechanism

**One canonical concept per aligned pair; redefine the other side's ENTRIES to denote it.**

The alignment is a **layer above both lexica** — it must be, and this is not a stylistic choice:
Rule 22 requires references to resolve *same-or-lower*, so an importer-side lookup table would need
UMLS to load *below* WordNet **and** the alignment to already exist before the import that produces
it. Circular. A layer above both redefines `umlscui:e_*` (lower) and references `wn:n*` (lower) —
legal, and the load order (WordNet chain, then UMLS chain) already puts both beneath it.

For each accepted pair `(C1442792, n00024720)` the layer emits:

```
resource umlscui:e_C1442792_0 : lexicon:LexicalEntry {
    lexicon:form     = "state";
    lexicon:cat      = type_expr( lexicon:cat_n(wn:n00024720, lexicon:num_any) );  // was umlscui:
    lexicon:sem      = wn:n00024720;                                               // was umlscui:
    lexicon:sense    = "wn:state.n.00024720";                                      // was "umls:…"
    lexicon:sem_type = type_expr( Set );
    lexicon:grade    = epistemic:declared;
    lexicon:in_lexicon = lexicon:umls;
}
```

**Canonical side = WordNet.** Not arbitrary:
- WordNet nouns carry a **deep `@` hypernym taxonomy**; UMLS concepts are depth-2 (`CUI → TUI →
  Entity`). Canonicalizing to WordNet **preserves the subsumption lattice the parser already uses**.
- It adds **zero new `subclass_of` edges**. That is the hard lesson of 2026-07-11: adding a
  supersense parent to WordNet's noun classes and emitting the UMLS TUI ISA tree changed the
  subtyping lattice and **broke parses** (the branch was reverted). **The alignment must be inert
  for the lattice** — it changes only which class an *entry* denotes.

**What is deliberately NOT done:** no new classes, no `owl:sameAs`-style equivalence axiom, no
merging of the class hierarchies, no touching `MRHIER`/`MRREL`. Identity of *entries*, not surgery
on the type lattice.

### 2a. The one open question — settle it with a spike, before building the pipeline

Two entries now share `(form, cat, sem, sense)`. **Does `lookup_span` dedupe them, or does it keep
both?**

- **If it dedupes:** the duplicate never enters the chart, and **a cap slot is freed** — the 47%
  becomes available for a genuinely different sense. This is the big win.
- **If it keeps both:** the two readings are now `Exp`-equal, so `subsume_duplicates` collapses them
  *after* parsing. Readings still drop — but the cap slot stays wasted, and the chart still built
  the duplicate.

The second is worth having; the first is worth much more. **Read `lookup.rs` and find out before
writing the adjudicator** — the answer decides whether dedup-at-lookup needs to be added.

---

## 3. Candidate generation

**Same surface string, both sides glossed** → **40 065 pairs** (UMLS concept × WordNet noun synset).

**No pre-filter.** The obvious one — require the UMLS semantic type and the WordNet supersense to
agree — was measured (5-fold cross-validated) and is **too lossy to be a gate**:

| filter | duplicates kept | LLM work cut |
|---|---|---|
| exact TUI ↔ supersense | **74%** | 61% |
| STN branch depth 1 | **93%** | only 23% |

To keep 93% of duplicates it removes only 23% of the work; to remove 61% it **discards a quarter of
the duplicates** — silently, and a dropped duplicate is one we never merge. 40 065 pairs is
batch-tractable for an LLM directly. Keep TUI/supersense agreement as a **scoring feature**, never
as a gate.

---

## 4. Adjudication

**Ask an LLM: do these two glosses name the same concept?** Inputs: the surface, both glosses, both
type labels (UMLS semantic type + WordNet supersense) as features. Output: `same | different |
unsure`, with a confidence and a one-line reason.

**Validation set, already built: 330 gold pairs** — same surface, normalized gloss token-Jaccard
≥ 0.75. These are near-certain duplicates. **The adjudicator must recover them**; measure precision
and recall against this set *before* trusting it on the rest.

**Temperature 0**, and — the lesson of 2026-07-11 — **record every verdict**. `temperature: 0` is
*not* deterministic: two live runs of the sense reranker differed on **5% of the capped top-2**. The
adjudication must be a **recorded artifact** (`alignment.json`), not a live call at build time, or
the lexicon is irreproducible.

**Coverage limit, stated up front:** only **10.6%** of UMLS CUIs have an `MRDEF` gloss. That sounds
fatal and isn't — **all four of the corpus's witnessed duplicates are glossed** (`events`, `genes`,
`DNA repair`, `cell death`). Prose uses the well-described concepts; the un-glossed 89% is a long
tail of source-specific codes that never surface in text. **Do not engineer for that tail before
measuring whether it appears.**

---

## 5. Measurement — and why it is now honest

The A/B that today's harness makes possible, and which was impossible before it:

```bash
scripts/measure-parse-rate.sh --replay results/<baseline-run>/ranks.json
```

**Replay the recorded rankings against the aligned lexicon.** The LLM is held fixed, so any delta is
**the alignment and nothing else**. Without this, the reranker's run-to-run drift (5% of cap
decisions) would swamp the effect.

⚠️ A replay whose lexicon changed will **MISS** — the recorded key includes the candidate sense-set,
and alignment changes it by construction. So the honest protocol is:

1. **Live reranked run** on the aligned lexicon → new `ranks.json`. (The `misses` counter tells us
   how much the candidate sets moved — itself a measure of the merge.)
2. Compare against `baseline.json` with `scripts/eval-parse-rate.sh --baseline`.
3. Replay is for **iterating on the parser** with the lexicon fixed, not for this A/B. Do not
   misuse it and claim reproducibility it doesn't have.

### Gates

| gate | condition |
|---|---|
| **Coverage — non-negotiable** | `grammar-gap 0`, `missing-lexeme 0`. A merge that closes a reading is a regression, not a win. |
| **The mechanism fired** | the cross-lexicon-pair rate in `ranks.json` drops from **47%** |
| **The payoff** | `sense×` falls from median **5.5** (rerun the ambiguity factoring) |
| **The goal** | `encoded` rises from **1/62** |

`encoded` may *not* move even if `sense×` falls — the structural axis (median 6 skeletons) also
multiplies, and perfect sense collapse alone leaves ~18% of readings. **Say so rather than dressing
up a partial win.**

---

## 6. Risks

- **Over-merging.** A false `same` fuses two genuinely different senses and can *destroy* the
  correct reading → a grammar gap. The coverage gate catches it; the gold set bounds its rate.
- **Losing UMLS typing.** Canonicalizing to WordNet means an aligned word no longer carries its TUI.
  Countability is decided at *import* time and baked into the entries (the additive
  `cat_n(C, mass)`), so preserve the mass/count variants when redefining — **verify, don't assume**.
- **Lattice drift.** If it ever seems necessary to add a `subclass_of` edge to make this work,
  **stop**: that is precisely what broke the parses on 2026-07-11.

---

## 7. Order of work

1. **Spike (§2a)** — does `lookup_span` dedupe identical entries? Decides the design. *Hours.*
2. **Candidate + adjudication pipeline** → `alignment.json`; validate against the 330 gold pairs.
3. **Alignment-layer emitter** → redefine the UMLS entries; reseed.
4. **Measure** — coverage gate first, then pair-rate, `sense×`, `encoded`.
