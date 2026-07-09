# D63 — Parse-gap closure for the test document (full-lexicon baseline + plan)

**Roadmap (four phases, in order).** The parse→encode pipeline closes out in four phases, worked in
sequence (the user's directive, `2026-07-06` — stop detouring):

| phase | status | measure |
|---|---|---|
| **1. OOV / lexical** | ✅ **closed** (`2026-07-05`) | `missing-lexeme 0`, distinct OOV 0 — Stage-A augmentation grounds the whole page |
| **2. Parsing gaps** | ✅ **grammar-complete** (`2026-07-08`) | reranked grammar-gap **12 → 5**; the 5 residual are **search-starvation** (every construction parses in isolation), not missing grammar — **§0** below |
| **3. Ambiguity / search** | 🔵 **next** | the 5 residual gaps + every closing unit AMBIG (**0 ENCODED**); one root cause — the beam / mass-shim over-generation (§6) |
| **4. Performance** | 🔵 **next** (folds into 3) | 62 units in 74 min; pathological outliers (up to 930 s) on ambiguity-rich units — same root cause |

Phases **3 and 4 share one root cause — the mass-shim over-generation** (Step 4 RC-1 head-inheritance is
loose): killing the spurious `mass` readings collapses ambiguity *and* parse time together. A real
intermediate-cell beam is the backstop. (§6 / §7.)

---

## 0. Grammar gaps CLOSED — reranked config (Derived, `2026-07-08`)

**All `--no-llm` counts in this note are cap-only** (static most-frequent-sense first) — the **wrong
measurement config**, which inflates the grammar-gap count. The canonical config attaches the contextual
sense reranker: `cargo test --release -p eigenius-wordnet --features use-llm --test db_backed_encoding
wrn_first_page_over_full_lexicon -- --ignored --nocapture` with `ANTHROPIC_API_KEY` set, over the snapshot
`db-snapshot/wordnet-umls-all-2026-07-08` (`--umls-all`, 52 layers, 2.5 G, all parsing-fixes; the
`DEFAULT_SNAPSHOT`). Under it the grammar-gap count fell **12 → 5**, and **the 5 residual are
search-starvation, not grammar**:

| step | grammar-gap | what changed | commit |
|---|---|---|---|
| cap-only baseline (`--no-llm`) | 12 | the §-below `2026-07-06` measure | — |
| + reranker (`--features use-llm`) | 9 | contextual sense order surfaces non-frequent senses under the cap (s20 GAP→CLOSED×144) | — |
| + static-rank widen fallback | 8 | the reranker is **non-monotonic** — it can bury a construction-triggered category variant that is not a distinct content sense; on an all-known-vocab gap, retry once with `ranks=None` (`widen_unpacked` / `widen_packed`) | `7d9cda4` |
| + gap #1 (bnp compound-kind subject) | 7 | bare compound-kind subjects shift via the `bnp` unary rule (`bare_nominal_shifts` = kind-subject + bare-plural + bare-mass) | `970e9ae` |
| + #5 linking-verb + #2 UMLS process-mass | 5 | WordNet frames 6/7 → `FrameKind::LinkingAdj` (copula-adjective `(S[dcl]\NP)/(S[adj]\NP)`); UMLS process/function TUIs (T038–T046 / T067 / T070) → mass (`concept_is_mass` semantic-type shim) | `1cbeeda`, `ab6a909` |

**The 5 residual gaps are all search-starvation** — every construction parses in isolation; the full
sentence exceeds the beam / cell budget under full-sentence sense-crowding:

| # | sentence (head) | construction (parses in isolation) |
|---|---|---|
| #3 | `Some MSI lines … were represented by …` | passive + complex agent (RC row below) |
| #4 | `… identified WRN as … compared to …` | `V X as Y` (RC-3) + nested verb+PP |
| #7 | `MSI cell lines … showed greater dependence … than …` | phrasal comparative (RC-2, construction CLOSED `2026-07-07`) |
| #8 | `These lines possess events that are predictive of …` | adjective + PP-complement (RC-4) |
| #9 | `… suggest that WRN dependency is not simply a result of …` | clausal complement + negated copula (RC-8) |

**Conclusion — grammar-gap work is DONE for this page.** The grammar covers every construction on the
page; the remaining 5 gaps are the beam not reaching the (existing) parse under full-sentence
sense-crowding. Phases 2 / 3 / 4 have collapsed into one lever: **scale the search** — the same beam /
mass-shim over-generation root cause (§6 / §7; D63 §8.7 / GH#97). The reranked run also carries handled
`felicity_readback` `catch_unwind` panics on ill-typed resource-as-function candidates (documented, not
contamination).

---

**Full re-measure (Derived, `2026-07-06`, snapshot `wordnet-umls-all-2026-07-06`, page cnl-v2, `--no-llm`,
74 min — cap-only, superseded as ground truth by §0):** 62 units → **ENCODED 0, AMBIG 50, GRAMMAR-GAP 12,
MISSING 0, OPEN 0, SCALE-BOUND 0** — **81% close**, up from 68% (AMBIG 42 / gap 20) at the Step-9 baseline
(§1b). Step 5 (apposition) + 5b (comma inheritance) + 5c (coordination refactor to core-en's
list-with-operator shape) closed **8 gaps**; the corpus's own `…the MMR genes MSH2, MSH6, PMS2 or MLH1
cause Lynch syndrome` now parses (AMBIG×240).

### Phase-2 backlog — the (historical) cap-only 12-gap list — SUPERSEDED by §0

*Historical, cap-only ordering. The **construction diagnoses below stand**; the counts do not — §0 is the
authoritative reranked status (grammar-gap 5, all search-starvation). Rows kept for the per-construction
fix record.*

| RC | # | construction — sentence(s) |
|---|---|---|
| **RC-2** phrasal comparative (`greater/fewer X … than Y`) | 2 | `greater dependence on WRN than …`; `fewer … than typical lineages` → [d63-comparative-phrasal.md](d63-comparative-phrasal.md). **CLOSED (`2026-07-07`).** **FAIL-CLOSED CORRECTION (witnessed):** ~~Attributive comparatives already parse~~ was FALSE — `a larger cell line` gapped (positive `a large cell line` closed); comparative adjectives had only the predicative `(S[adj]\NP)/cat_pp_than`, no `N/N`. FIXED: a bare `S[adj]\NP` reading `λx. gt(deg(x), deg(anaphor))` — the elided `than`-standard is an **anaphoric hole** (discourse-relative, d63-comparative-phrasal §8) → `a larger/stronger X` parses **OPEN**, one comparison-standard hole the D64 resolver fills. Demo + importer (`cmp_attrib_sem`, ~18.9k emitted); test `attributive_comparative_opens_with_a_standard_hole`. |
| **modal + coordinated object** | 1 | `WRN dependency **may require** specific lineages or a stronger mutation phenotype` (s20). **CLOSED (`2026-07-07`)** — two grammar blockers, both fixed and composing → s20 parses **OPEN** (D64 fills the standard hole): **(1)** the attributive comparative `a stronger mutation phenotype` (RC-2 row); **(2)** coordinating type-raised **object-GQs over DIFFERENT noun types** (`lineages` ⊕ `phenotype`) — the object-GQ categories differ only in the exposed slot `cat_np(T)`, so coordination widens it to `common_super` (`common_cat` in category.rs, gated to backward-headed object-GQs in `coordinate_prop`) while the **per-disjunct semantics keep the distinct types**: `∃g:Gene.V(g) ∨ ∃c:CellLine.V(c)`. Witnessed (demo proxy `HeLa may affect a gene or a larger cell line`): `Possible(Or(∃:Gene …, ∃:(Σ CellLine. gt(deg, $anaphor$)) …))`, one hole. Tests `heterogeneous_object_gq_coordination_generalizes_type`, `s20_shape_parses_open_with_modal_coordination_and_comparative`. **NB: earlier calls of this residual "heterogeneous NP coordination" / "compound" were wrong** — it's type-raised object-GQ coordination; base-type-mismatch was the tell. **Corner (logged):** widening the exposed slot relaxes a type-restricted verb's selectional check to the supertype — over-generates for restricted verbs (rare); general verbs are exact. Bare-name ⊕ raised-GQ (`BRCA1 or a gene`) still gaps — a separate shape s20 doesn't use. |
| **RC-8** clausal complement + multiword verb | 2 | `hypothesized that … give rise to`; `suggest that … is not simply a result of` |
| **deep verb+PP / nested-PP** | 2 | `arises from hypermethylation of the MLH1 promoter`; `compared favourably to … biomarkers for …` |
| **RC-3** `V X as Y` | 1 | `identified WRN as the top … dependency` |
| **RC-4** adjective + PP-complement | 1 | `events that are predictive of MMR deficiency` |
| **RC-5** linking verb + predicate | 1 | `remained true with …` |
| **RC-7** copula-kind on compound subject | 1 | `Nucleotide repeat regions are microsatellites` |
| **passive + complex agent** | 1 | `were represented by these screening data sets` → [d63-passive-voice-handling.md](d63-passive-voice-handling.md) (general passive infra: object→subject promotion + `by_arg` + `rel(theme, ground)`) |

**No next parsing gap — the grammar is complete for this page (§0).** Every construction above now parses
in isolation: RC-2 comparatives and the s20 modal+coordinated-object gap CLOSED `2026-07-07`; RC-5
linking-verb (`1cbeeda`), RC-7 copula-kind on compound subjects (`970e9ae`), and the UMLS process-mass gap
(`ab6a909`) CLOSED `2026-07-08`. The five gaps that survive in the reranked full-page measure (§0:
#3/#4/#7/#8/#9) fail because the full sentence overruns the beam / cell budget, not because a rule is
missing. The whole remaining backlog is the **search-scaling lever** (§6 / §7; D63 §8.7 / GH#97).

What landed since the baseline below (all Derived, `2026-07-05`):
- **Stage-A augmentation is now injected into the parse.** The `LexiconBacked` transducer
  (`augment_lexicon_backed`, D63 lexicon-augmentation §6a) grounds OOV atoms against the form/description
  text indexes, and a new **`LexicalIndex` document-augmentation overlay** (`with_document_augmentation`)
  seeds those groundings alongside the persisted index — so the parser SEES them, uncommitted, over a
  DB-backed head. This resolves the §5 "no doc glossary in this run" caveat.
- **`recq` grounds** (→ `umlscui:C0084304`) via the form-text-index; **the compound adjectives close via
  the shipped morphology** (`double-stranded`, `hypermutable`, `pcr-based` — §2), `pcr-based` once its base
  `pcr` (C0032520, **T063**) is present under `--umls-all`.
- **Corpus coverage widened to UMLS Level-0 full** (`--umls-all`, all 127 semantic types →
  `wordnet-umls-all-2026-07-05`, 2.4G): concepts outside the prior WRN-relevant TUI subset — e.g.
  `wilcoxon` (→ `umlscui:C0242931` "Wilcoxon Rank Test", T081) — now ground. The subset's OOV residuals
  were **coverage**, not grounding/morphology defects.

Sequence this note sits in (the four-phase roadmap above): **OOV ✓ → parsing gaps (this note, active) →
ambiguity (§6) → performance (§7)** → then the grading-phase gaps ([d63-next-steps.md](d63-next-steps.md)).

---

## 1. The measurement (reproducible)

```
scripts/measure-parse-rate.sh --no-llm          # page: cnl-v2, deterministic (no reranker)
```

- **Page:** `references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v2.txt` (WRN-Helicase Nature
  first page, controlled-English v2 rewrite; 4 paragraphs, ~616 words, **62 units**).
- **Lexicon:** full WordNet (`--all`) + UMLS (all types), snapshot `wordnet-umls-all-2026-07-03`
  (manifest-consistent with HEAD — the only bootstrap-ontology change since is the committed `kind_of`
  axiom; no ManifestDrift SKIP).
- **Deterministic:** `--no-llm` (cap-only, no sense reranker) — the clean parse-gap baseline; the
  reranker bears only on *ambiguity* (step 3), not on whether a unit parses at all.
- **Harness:** `wrn_first_page_over_full_lexicon` (`crates/eigenius-wordnet/tests/db_backed_encoding.rs`).
  **As of `2026-07-05` it runs Stage-A augmentation** (`augment_lexicon_backed`) and overlays the
  groundings onto the index (`with_document_augmentation`) before parsing — so OOV atoms the base lexicon
  misses are grounded + seeded, not gapped. The pre-augmentation baseline below is the *raw* page over the
  base lexicon (no injection); see §1a for the re-measure with augmentation.

**Result line — PRE-augmentation baseline (Derived — verbatim, snapshot `wordnet-umls-all-2026-07-03`):**

```
WRN first page over FULL lexicon: 62 units → encoded 0, ambiguous 39, open 0,
                                  missing-lexeme 6, grammar-gap 17, scale-bound (known, >60 tok) 0
distinct OOV tokens (4): {"double-stranded", "hypermutable", "pcr-based", "recq"}
OOV by fix-bucket: domain-lexicon 4, connectives/function-words 0, -ly adverbs 0, stat-symbol leaks 0
```

| class | count | share | reading |
|---|---:|---:|---|
| **ambiguous** | 39 | 63% | parses (multiple readings) — a *win* for "does it parse"; ambiguity is step 3 |
| **grammar-gap** | 17 | 27% | all words known, no parse — the real grammar/frame gaps |
| **missing-lexeme** | 6 | 10% | blocked by an OOV token |
| encoded | 0 | 0% | nothing yet parses to a *single* clean reading (sense-crowding) |
| open | 0 | 0% | reshape closed these (was 35) |
| scale-bound | 0 | 0% | Lever B beam kept every unit under the 60-tok cap |

**"Parse completely" target = the 23 gap-units** (6 missing-lexeme + 17 grammar-gap). The 39 ambiguous
units already parse.

### 1a. Re-measure WITH Stage-A augmentation (Derived, `2026-07-05`, snapshot `wordnet-umls-2026-07-05`)

Same page + harness, now with the augmentation injected (`augmentation: 9 OOV grounded + injected, 1
residual`):

```
WRN first page over FULL lexicon: 62 units → encoded 0, ambiguous 38, open 0,
                                  missing-lexeme 2, grammar-gap 22, scale-bound 0
distinct OOV tokens (1): {"pcr-based"}
```

**Missing-lexeme 6 → 2; distinct OOV 4 → 1.** The augmentation eliminates OOV as a blocker: `recq`,
`double-stranded`, `hypermutable` all resolve, so their units re-bucket **out of missing-lexeme**. They
land in grammar-gap (17 → 22), not ambiguous — i.e. the OOV was *masking* an underlying **grammar** gap
(the construction / subject-typing gaps of §3); closing the lexeme reveals it. So the augmentation does not (by
itself) lift the parse rate — it converts "blocked by an unknown word" into "blocked by a missing frame",
which is the honest state: **the residual is grammar, not lexicon.** The sole remaining OOV, `pcr-based`,
needs its base `pcr` — absent from this WRN-subset snapshot (T063), present under `--umls-all` (§1b).

### 1b. Over `--umls-all` (Derived, `2026-07-05`, snapshot `wordnet-umls-all-2026-07-05`)

The full Level-0 corpus closes the last OOV and the coverage gaps (probe `probe_wilcoxon_pcr_grounding`,
`wrn_page_oov_closure_deterministic`):
- `has_token("pcr") = true`, `has_token("pcr-based") = true` — the shipped `X-based` rule fires once the
  base is present; `pcr-based` is no longer OOV.
- `wilcoxon` (subset-OOV, T170/T081 outside the WRN TUIs) grounds → `umlscui:C0242931` "Wilcoxon Rank
  Test".
- Deterministic OOV closure over the *original* (non-CNL) page: baseline 13 → 16 grounded → **1 residual**
  (`0.56-fold`, a numeric-fold compound — the numbers path, a separate known gap).

**Full 62-unit parse re-measure over `--umls-all` (Derived, `2026-07-05`; the Step-9 run, ~38 min):**

```
WRN first page over FULL lexicon: 62 units → encoded 0, ambiguous 42, open 0,
                                  missing-lexeme 0, grammar-gap 20, scale-bound 0
distinct OOV tokens (0): {}          augmentation: 1 OOV grounded + injected, 0 residual OOV
```

**The lexical side is fully closed — `missing-lexeme 0`, distinct OOV `0`** — and it lifts the parse rate:
**ambiguous 42** (vs 38 subset / 39 pre-aug baseline), because with full coverage the ex-OOV units parse
rather than re-bucketing to grammar-gap. The three-way progression:

| metric | pre-aug baseline | subset + aug (§1a) | `--umls-all` + aug |
|---|---:|---:|---:|
| **ambiguous** (parses) | 39 | 38 | **42** |
| grammar-gap | 17 | 22 | **20** |
| missing-lexeme | 6 | 2 | **0** |
| distinct OOV | 4 | 1 | **0** |

The **Step-9 gate is half met**: `missing-lexeme → 0` ✓; the **20 grammar-gaps** remain — the verb+PP /
construction / subject-typing gaps of §3 (steps 4–11), which are grammar, not lexicon.

Two meta-findings (Derived), both **out of scope here** (step 3):
- **0 encoded** — every parse is ambiguous (AMBIG ×8 to ×64). Sense-crowding; the reranker exists to
  collapse it.
- **Long sentences cost 3–5 min** — Lever B beam dropping *millions* of chart items on the 16–21-tok
  units (e.g. 325 s on unit 47). A perf concern; nothing hit the hard scale bound.

---

## 2. Gap class 1 — OOV — CLOSED (`2026-07-05`)

The 4 baseline OOV tokens all resolve — 3 via the shipped compound morphology
([d63-compound-morphology.md](d63-compound-morphology.md)), `recq` via the form-text-index augmentation.
Each closure is Derived (§1a run + `probe_wilcoxon_pcr_grounding`):

| token | units | shape | resolution |
|---|---|---|---|
| `double-stranded` | 15 | hyphen compound-adj | compound morphology (Slice 1, hyphen-head) — known |
| `hypermutable` | 21 | `hyper-` + adjective | compound morphology (Slice 1, closed prefix) — known |
| `pcr-based` | 45, 49 | `X-based` denominal adjective | compound morphology (Slice 2, `X-based`) — known **once base `pcr` present** (T063; `--umls-all`, §1b) |
| `recq` | 48, 50 | gene-family name | form-text-index augmentation → `umlscui:C0084304` (grounded alias, overlaid) |

So "domain-lexicon OOV entry" was the wrong frame for 3 of the 4 — they are **productive derivations** the
morphology decomposes, not per-word entries; and `recq` is grounded to an existing UMLS concept, not
minted. The only thing the subset was actually missing was *concept coverage* (`pcr`), which `--umls-all`
supplies.

---

## 3. Gap class 2 — grammar-gap (20 over `--umls-all`): the verb+PP frame is FIXED; the residual is subject-typing + missing construction rules

Every token in all 20 is **known** (`has_token=true`, incl. the UMLS terms — `msi`, `wrn`, `lynch
syndrome`, …) — these are purely *grammatical*. Short isolation probes over the augmented `--umls-all`
index, run **both** deterministic (`--no-llm`) and with the **live reranker** (`--features use-llm`),
localize each blocker to its construction, not the beam (all **Derived**, `probe_grammar_gap_root_causes`,
`2026-07-05`):

| probe | det | reranker | reads as |
|---|---|---|---|
| `instability contributes to cells` | CLOSED×11 | CLOSED×4 | **verb+PP frame WORKS** |
| `MSI contributes to cells` | GAP | GAP | bare UMLS subject fails |
| `MSI results from deficiency` | GAP | GAP | " |
| `cells respond to therapy` | CLOSED×36 | CLOSED×12 | **verb+PP frame WORKS** |
| `MSI is associated with responses` | CLOSED×16 | GAP | det-only artifact; subject fails |
| `MSI occurs in cancers` / `MSI arises from deficiency` | GAP | GAP | bare UMLS subject fails |
| `cells showed greater dependence than counterparts` | GAP | GAP | no comparative-`than` rule |
| `cells contained fewer mutations than lineages` | GAP | GAP | " |
| `we evaluated MSI as a biomarker` | GAP | GAP | no `V X as Y` rule |
| `regions are microsatellites` | CLOSED×2 | CLOSED×2 | copula-kind WORKS (simple subj) |
| `nucleotide repeat regions are microsatellites` | GAP | GAP | copula-kind fails (compound subj) |
| `WRN requires lineages or a phenotype` | GAP | GAP | object coordination |
| `classifications were concordant with phenotyping` | GAP | GAP | no adjective+PP-complement |
| `findings remained true` | GAP | GAP | no linking-verb+predicate |

**Two headline corrections to the earlier analysis (both Derived):**

1. **The verb+PP-complement frame (§4 steps 1–2) is FIXED and validated over `--umls-all`.**
   WordNet-noun subjects compose — `instability contributes to cells`, `cells respond to therapy` CLOSED
   under both configs. "Missing verb+PP frame" is **no longer** the dominant grammar-gap.
2. **The residual "verb+PP" gaps are the SUBJECT, not the verb — and NOT sense-crowding.** A bare
   UMLS-sourced form (`MSI`) as a finite-verb subject GAPS under **both** the deterministic cap and the
   reranker — the controlled contrast `instability`✓ / `MSI`✗ (only the subject differs) isolates it to
   the subject, and the reranker even *removes* the one deterministic "win" (`associate`, CLOSED×16 → GAP),
   so it is a real compositional gap, not a beam artifact. **Leading mechanism (Declared, to confirm):**
   UMLS concepts seed as **count** common nouns, and a bare count noun can't be a subject — whereas WordNet
   `instability` is **mass** (bare-shifts to a subject NP) and the note's glossary ACCEPTANCE (§4 step 2)
   worked precisely because it grounded MSI to the **mass** concept `C0920269`. Confirming probes next
   round: `the MSI contributes to cells` (add a determiner) + inspect the UMLS entry's `cat_n` `num` feature.

### Root causes (witnessed) and the 20 sentences

A sentence can carry >1 blocker (**primary** first). Probed causes are Derived; un-probed ones are
Declared-by-construction (flagged `?`).

| RC | root cause | evidence | sentences (primary) |
|---|---|---|---|
| **RC-1** | **Bare UMLS-noun subject** — the verb+PP frame is fine; a bare UMLS term can't be a finite-verb subject (mass-vs-count) | Derived (contrast, both configs) | `MSI results from…` (3), `MSI contributes to…` (5), `MSI occurs in…` (6), `MSI arises from…` (7), `MSI is associated with…` (10) |
| **RC-2** | **No comparative `than`** construction | Derived (GAP both) | `…greater dependence…than…` (18), `…fewer mutations…than…` (19); `compared to…` in (14) |
| **RC-3** | **No `V X as Y`** predicative small-clause | Derived (GAP both) | `evaluated MSI as a biomarker` (16); `identified WRN as…` (14) |
| **RC-4** | **No adjective + PP-complement** (`concordant with X`) | Derived (GAP both) | `…concordant with…and with…` (12) |
| **RC-5** | **No linking-verb + predicate** (`remain/stay true`) | Derived (GAP both) | `…remained true with…` (15) |
| **RC-6** | ~~Coordination in quantified / apposed / mismatched-NP contexts~~ → **apposition FIXED** (Step 5); residual = **comma-list with final `or`** (Step 5b) | Derived (apposition CLOSED×8/12/16/44/60; comma-`or` list GAP) | ~~`the MMR genes MSH2,…or MLH1` apposition (8)~~ FIXED; residual `MSH2, MSH6, PMS2 or MLH1` comma-`or` list. (quantified 13, proper-noun 14, mismatched-NP 20 already CLOSE) |
| **RC-7** | **Copula kind-predication on a *compound* subject** (`are_kind` fires on simple, not 3-word, subjects) | Derived (simple✓/compound✗) | `Nucleotide repeat regions are microsatellites` (4) |
| **RC-8** | **Clausal complement + multiword verb** (`hypothesize that`, `give rise to`) | Declared (note's prior `give rise` beam finding) | `We hypothesized that…would give rise to…` (1) |
| **?** | **verb+PP frame *or* deep object/nested-PP** — non-bare-UMLS subject; needs a probe | Declared | `queried dependencies in cancers with MSI` (2), `arises from hypermethylation of the MLH1 promoter` (9), `compared favourably to…` (17); deep object `…responses to immune checkpoint blockade` (10, 11) |

**Named entities are no longer a bucket.** `Lynch syndrome` / the gene symbols are `has_token=true` over
`--umls-all` (loaded UMLS concepts) — where they appear (7, 8) the blocker is RC-1 (subject) or RC-6
(apposition), not named-entity seeding. The old "Named disease (F)" bucket is closed by coverage.

---

## 4. The step-by-step plan (leverage order)

- [x] **Step 1 — verb+PP-frame root cause: DIAGNOSED (importer-side, frame-specific).** Witnessed
      (code + live probe `non_pp_verb_rejects_a_pp_complement`, `2026-07-04`):
  - The WordNet importer (`convert.rs::classify`) has **no verb+PP-complement category**: it emits only
    Intransitive `S\NP` / Transitive `(S\NP)/NP` / Ditransitive / Clausal, and maps PP-oblique frames
    **coarsely** — 12/13/20/21/27 → transitive (preposition dropped, *bare NP* expected), 4/22 →
    intransitive (PP dropped). A documented "stage-1 loss".
  - Prepositions **are** seeded with both a VP-adjunct `(S\NP)\(S\NP)/NP` and a noun-mod `cat_pp/NP`
    entry (`closed-class.esl`) — so a PP *can* attach; the gap is verb-side.
  - **Category fact (live):** a transitive `(S\NP)/NP` verb takes a bare NP and cannot consume `prep + NP`
    (a `cat_pp`): `HeLa affects to BRCA1` → 0 parses; `HeLa affects BRCA1 in HeLa` → 2 (PP adjoins after
    the object). And `*affects to BRCA1` **should** gap — `affect` is not a PP verb.
  - So an **argument-PP** verb (`contribute to`, `result from`, `respond to`, `associate with`,
    `depend on`) — subcategorized for the PP but emitted transitive — gaps: `contributes to cancers`
    wants a bare NP but gets `to cancers` (a PP). **This is the bug.**
  - **Refinement:** the two *adjunct-PP* verbs (`occur in`, `arise from`) stand alone; their PP should
    VP-adjoin already, so their corpus gaps are likely the object (`Lynch syndrome`, coordination), not
    the verb frame — re-check after the fix (they may re-bucket out of "verb-frame").
- [x] **Step 2 — the fix: a frame-specific verb+PP-complement category (`cat_pp_arg`).** Mirrors the
      comparative `cat_pp_than` (an argument-PP whose ⟦·⟧ = Entity). A verb subcategorizing for a PP is
      `(S\NP)/cat_pp_arg`; a **transparent argument-marker** `to/from/on/with = cat_pp_arg/NP` (sem `λy. y`)
      exposes the object. A distinct `cat_pp_arg` (not a bare NP) forces the preposition, so a plain
      transitive verb `(S\NP)/NP` (`affect`) still rejects `to X`. Same `Entity→Entity→Prop` sem_type as
      transitive → felicity gate unchanged.
  - [x] **Grammar half — DONE + validated (`2026-07-04`, no reseed; bootstrap recompiles fresh).**
        `cat_pp_arg` declared (`lexicon-ontology.esl`) + denoted (`category.rs`); argument-marker prep
        entries (`closed-class.esl`); the `GqPrepObj` parser rule extended (3-way `PrepObj`) so the marker
        feeds a **raised GQ** (bare-plural/kind object) → the object entity `Q(prep_sem)`. Test
        `argument_pp_verb_parses_verb_prep_object`: `HeLa contributes to BRCA1` (individual) **and**
        `HeLa contributes to genes` (bare-plural **kind**, sem has `kind_of`) parse; `affects to BRCA1`
        gaps (guard `non_pp_verb_rejects_a_pp_complement`). Full kernel suite + clippy green.
  - [x] **Importer half — DONE + committed (`2026-07-04`; grammar `f9859fd`, importer `2b22705`).**
        `convert.rs`: added `FrameKind::PpOblique` (cat `(S\NP)/cat_pp_arg`, sem_type `Entity→Entity→Prop`);
        `classify` routes the **single-PP** frames **{4, 12, 23, 27}** to it. Obj+PP frames (13/20/21/22)
        stay coarse; frame 14 stays ditransitive (a mis-route the importer test `frame_classification_*`
        caught — my recollection of frame 14 was wrong). Reseeded → snapshot `wordnet-umls-2026-07-04`
        (7,398 `cat_pp_arg` entries). Confirmed emitted: `contributes:(S[fin]\NP_sg)/cat_pp_arg`.
  - [x] **ACCEPTANCE VERIFIED (glossary path).** `measure_abbreviation_glossary` over the snapshot:
        `MSI contributes to several cancers` **base=GAP → glossary=CLOSED×8**, sem
        `v02324478_p(kind_of(Σ…cancer…), kind_of(C0920269))` — the `_p` (PpOblique) verb + `MSI` grounded to
        the mass concept `C0920269`. The verb+PP fix and the Stage-A glossary **compose**. (3/6 MSI
        sentences recovered as closed kind-predications; `MSI can arise from Lynch syndrome` still gaps —
        named-disease bucket.) The verb+PP composition itself is also confirmed lexicon-wide (isolation,
        `--no-llm`): `instability contributes to cells` → AMBIG; `MSI contributes to cells` → GAP (only the
        subject differs).
  - **Observation (not a task) — raw parse-rate (`--no-llm`, whole page): 17 → 18 grammar-gap, a beam
        artifact, not a real loss.**
        No raw gaps closed: every verb+PP sentence in the doc has an `MSI`/abbreviation subject, so it needs
        the glossary (above) to subject-ify. One regression — `We hypothesized … would give rise to …`
        flipped AMBIG→GAP: `give rise`'s multiword cat is unchanged; standalone `rise` gained a competing
        `to`-verb reading (`(S\NP)/cat_pp_arg`) that at beam=512 crowds out the winning derivation
        (1.0s→84.3s). The **live reranker parses it** (AMBIG×256, 26s) — so the raw regression is absent
        under the operational reranked config.
  - **UPDATE (`2026-07-05`, `--umls-all` battery, §3): the "sense-crowding" implication was WRONG.** The
        `MSI`-subject verb+PP sentences GAP under **both** the deterministic cap **and** the reranker —
        the reranker does *not* recover them (it even removes the one det-only win). So the blocker is not
        the beam; the frame is fixed and the residual is the **bare UMLS-noun subject** (RC-1, §3) — a
        real compositional gap (mass-vs-count), now the highest-leverage item (Step 4 below).
  - *Looseness (stage-1):* WordNet frames don't encode *which* preposition, so `cat_pp_arg` accepts any PP
    (`contributes in cancers` would also parse) — verb-specific but prep-generic; specific-prep is later.
  - [ ] **Step 2b (object+PP frames `((S\NP)/cat_pp_arg)/NP`) — folded into the new Steps 7/8 below.**
        Extend the verb+PP fix to the **object+PP** frames **{13, 20, 21, 22}** (`base X on Y`, `identify
        X as Y`, `----s something PP`) — currently routed coarsely to transitive (object kept, **PP
        dropped**) — emitting `((S\NP)/cat_pp_arg)/NP` (object, then the argument-PP). This is the same
        machinery the current 20 need for **`V X as Y`** (RC-3, Step 7) and the adjective **`concordant
        with X`** (RC-4, Step 8); it also resolves `based on X` (frame 21, [d63-compound-morphology.md
        §2a](d63-compound-morphology.md)).
- [x] **Step 3 — the 4 OOV: CLOSED (`2026-07-05`).** The 3 productive derivations resolve via the shipped
      compound morphology (**[d63-compound-morphology.md](d63-compound-morphology.md)** — `pcr-based` =
      `X-based` Slice 2, `hypermutable` = `hyper-X` / `double-stranded` = hyphen compound-adj, Slice 1),
      *not* per-word entries — as reframed. `recq` was **not** a minted named entity either: it grounds to
      the existing UMLS concept `umlscui:C0084304` via the **form-text-index augmentation** (D63
      lexicon-augmentation §6a), overlaid onto the parse index (`with_document_augmentation`). See §1a/§1b.
      Net: the missing-lexeme units re-bucket to grammar-gap (their tokens are now known), so this step
      lifts the *lexical* blocker, not the parse rate — the residual is the §3 grammar gaps.
**Re-ordered by leverage over the current 20 grammar-gaps (§3 RC counts), `2026-07-05`:**

- [x] **Step 4 — RC-1: bare UMLS-noun subject — FIXED + VERIFIED (`2026-07-06`).**
  - **Mechanism (witnessed, `probe_step4_bare_umls_subject`).** The grammar is correct; the lexicon was the
    gap. A bare **singular count** noun has no subject reading (`gene contributes` GAP); a **determiner**
    (`the MSI contributes` CLOSED×4), a bare **plural** (`genes contribute`), or a bare **mass**
    (`instability contributes`) all subject-ify. `MSI` was emitted count-only (`cat_n(C0920269, num_any)`,
    no `mass`), so it gapped exactly like bare `gene`; `instability` parses bare because the `--countability`
    lexicon mass-marks it. `MSI` is mass as a *corpus* fact (always "microsatellite instability", head
    `instability` = mass) — not a per-document one, so the alias model (OOV/in-doc-defined only) never
    re-typed it.
  - **Fix (A) — importer mass-shim by head-inheritance.** The UMLS importer emits an ADDITIVE
    `cat_n(C, mass)` for a concept whose preferred-name **head** is uncountable, reusing the shared
    `--countability` lexicon: [convert.rs](../../crates/eigenius-umls/src/convert.rs) `concept_is_mass` +
    `push_entries` (+ `Report.mass_entries`); a `--countability` flag on `umls-import`; the reseed script
    passes it. Never for named individuals (gene `cat_np`). General — replaces the removed 5-acronym hardcode.
  - **VERIFIED over `wordnet-umls-all-2026-07-06`** (2.5G; **893,872** additive mass entries): `MSI` now
    carries `cat_n(C0920269, mass)`, and every RC-1 sentence closes — `MSI contributes` ×4, `results from`
    ×3, `occurs in` ×4, `arises from` ×8, `is associated` ×72, `arises from Lynch syndrome` ×16
    (`probe_grammar_gap_root_causes`). RC-2/3/5/7 correctly stay GAP.
  - **Looseness (accepted; precision follow-ups):**
    - **Breadth — 894k entries (~16%).** The `head ∈ any-uncountable-sense` test over-fires on partly-count
      heads (`extension`, `finding`, …) and applies mass to *all* the concept's forms. Largely correct for a
      biomedical corpus (many concepts ARE mass phenomena), but it inflates ambiguity. **Follow-up:** a
      **strictly-uncountable** head test (uncountable AND no count sense) — general, sharpens the breadth and
      drops partly-count heads at once; needs a strictly-uncountable countability source.
    - **Acronym ↔ domain-word collision — filter (decision `2026-07-06`).** `gene contributes` closes via
      `GENE` = **G**ross **E**xtra-**N**odal **E**xtension (NCI, `TTY=SY`, `SUPPRESS=N` — a *valid* UMLS
      atom; head `extension` mass-shimmed). An acronym colliding with a **primary domain term** (`gene`) begs
      for misunderstanding, so it should be filtered. **Follow-up:** in the UMLS importer, suppress an acronym
      atom whose normalized form collides with a primary domain common noun — structural, not a per-atom
      blocklist; `SUPPRESS` doesn't catch these (all `N`). Distinct from the mass-shim — the spurious count
      entry pre-dated it (the mass reading is just the more-visible symptom).
  - **Note:** the mass-shim also flipped RC-4 (`concordant with`, CLOSED×206) and RC-6 (`requires … or`,
    ×56) to CLOSED — either mis-categorised mass-noun objects (real) or over-generation (the ×206 ambiguity
    is suspicious). The full-page `--no-llm` re-measure over `wordnet-umls-all-2026-07-06` (Step 12)
    re-baselines the grammar-gap count and settles which of the RC-2..RC-8 buckets actually remain.
- [x] **Step 5 — RC-6: coordination (DONE for apposition; comma-`or` list residual).** The Step-5
      re-measure (`probe_step5_coordination` over `wordnet-umls-all-2026-07-06`) obsoleted most of the
      original RC-6 framing: list/quantified/proper-noun/mismatched-NP coordination and sentence-13
      (`some MSI lines and some MSS lines were represented …`) already CLOSE. The one genuine
      construction gap was **close nominal apposition** (`the genes BRCA1 and MSH2 affect cells` — GAP).
    - **Fix (built + verified): `appose_group`** (`kernel/src/dcg/category.rs`) — a definite/bare
      common-noun HEAD (subject GQ `S/(S\NP_C)` or bare `cat_n(C,_)`) + a coreferential name-GROUP
      `cat_group(D,·,·)` passes the group THROUGH (the names specify the referents; the head classifies),
      gated on a **felicity** check. Seeded in the unpacked CKY (`lookup.rs`, keyed on head/group
      adjacency); the group then rides the existing `distribute` / `distribute_object` unchanged.
    - **Load-bearing finding — the felicity gate must be BIDIRECTIONAL.** Named individuals carry their
      broad UMLS **semantic type** (`umlssty:T028`), while a common noun carries its narrower **concept**
      (`umlscui:C0017337`, emitted `: umlssty:T028`, so `C0017337 ≤ T028`). The head is a SUBTYPE of the
      members' type — a one-directional `members ≤ head` gate rejected every real apposition. Gate =
      `⌊head⌋ ≤ ⌊group⌋ ∨ ⌊group⌋ ≤ ⌊head⌋` over the Σ-peeled base classes; still rejects a kind clash
      (`the cells BRCA1 and MSH2` → GAP, cell-concept and T028 subsume neither way).
    - **Verified over 07-06** (`probe_step5_apposition`): subject `the genes BRCA1 and MSH2 affect cells`
      CLOSED×8 (was GAP); bare `genes BRCA1 and MSH2 …` CLOSED×12; object `WRN affects the genes BRCA1
      and MSH2` CLOSED×60; **prep-object** `mutations in the genes BRCA1 and MSH2 cause cancer` CLOSED×16
      (no bridge needed — `distribute_object` generalizes to prepositions, both `fwd` functors);
      compound-Σ head `the MMR genes MSH2 and MLH1 …` CLOSED×44; felicity reject `the cells …` GAP.
      Fast regression: `closed_class_determiners.rs::close_apposition_*` (incl. a cross-importer
      granularity fixture reproducing the concept↔semantic-type typing).
    - **Residual (orthogonal, NOT apposition): comma-list NP coordination with a final `or`.**
      `MSH2, MSH6, PMS2 or MLH1` GAPS even bare — `coord_connective` hardcodes comma → `logic:And`
      (`reserved.rs`), so the commas build an `and`-group and the final `or` mismatches
      `coordinate_np`'s same-connective requirement (`colon, gastric AND ovarian`, all-`and`, works).
      The fix is a **neutral "list" comma** that the trailing `and`/`or` finalizes — a distinct
      coordination sub-case (new `Conn` semantics), tracked as **Step 5b**.
- [x] **Step 5b — comma-list connective inheritance (DONE, both paths).** A list comma is
      polarity-NEUTRAL: it inherits the list's FINAL explicit connective (`A, B, C or D` = all-`∨`,
      `A, B, C and D` = all-`∧`), not the hardcoded `and`. **Witnessed** (Derived) before design: the
      NP-group path GAPPED on comma-`or` (the same-connective guard); the prop-ending path silently
      MIS-parsed comma-`or` as `Or(And(a,b),c)` (fail-open).
    - **NP-group path**: a comma builds a neutral `conn_list` group ([`LIST_CONN`], a parser-internal
      sentinel — never a logic op nor a committed `Conn` ctor, so NO reseed); the trailing `and`/`or`
      REBINDS the whole group in `coordinate_np` (a `conn_list` left group accepts any op; a finalized
      left group still rejects `X and Y or Z` mixing). A never-finalized list defaults to `∧`
      (`group_conn_op`).
    - **Prop-ending path**: the comma no longer folds props binarily (it is `LIST_CONN`, which
      `coordinate_sem` can't fold); an **n-ary rule** in `parse_at_cap` gathers the comma-separated
      atomic conjuncts at the trailing `and`/`or` and folds them ALL with that one connective. Because
      the packed binary-hyperedge model can't express an n-ary fold, **comma-bearing sentences now
      route unpacked** (`parse_needs_unpacked`).
    - **Verified over 07-06**: `MSH2, MSH6, PMS2 or MLH1 affect cells` CLOSED×4 (was GAP); the corpus
      apposition `the MMR genes MSH2, MSH6, PMS2 or MLH1 affect cells` CLOSED×168 (was GAP); adjective
      `colon, gastric and ovarian cancers …` CLOSED (no regression); felicity reject holds. Fast
      regression `closed_class_determiners.rs::comma_list_inherits_the_final_connective` asserts all-`∨`
      / all-`∧` on both paths; full kernel lib 1610, `closed_class` 125 green.
    - **Residual (orthogonal — a cap/beam issue, NOT a construction gap).** The corpus PREP-OBJECT shape
      `mutations in the MMR genes MSH2, MSH6, PMS2 or MLH1 cause cancer` still GAPS, but every
      constituent parses in isolation: compound head + simple `and` in prep-obj CLOSED×116; plain head +
      comma-`or` in prep-obj CLOSED×48; the full compound + comma-`or` apposition in OBJECT position
      CLOSED×126. Only the maximal-ambiguity combination in the longest frame gaps — the signature of
      `DEFAULT_FOREST_CAP` (256) pruning the correct reading under the **mass-shim's ambiguity
      inflation** (Step 4 over-generation). Tracked with the mass-shim precision follow-ups + the full
      re-measure, not here.
- [x] **Step 5c — coordination refactor: the list-with-operator model (DONE).** Checking `Step 5b`
      against the reference grammar `references/openccg/grammars/core-en` (`conj.xsl` / `punct.xsl`)
      showed the shape it was ported from: ALL coordination is a deferred **linked list** with a single
      shared operator (`op-index-S`) — the comma is operator-neutral (`indexRel="Next"`, adds a list
      link), the conjunction sets the operator once, and per-category **list-completion** type-changing
      rules (`s-list` / `np-list-c/d` / `pred-adj-list`) close the list. Eigenius's NP path already did
      this (`cat_group` + `List` + `distribute`-at-verb); its PROP path folded EAGERLY (`coordinate_sem`
      → `And(a,b)` pairwise), which is the sole reason Step 5b needed the n-ary workaround + the
      `comma → unpacked` routing. This step aligns the prop path:
    - **`cat_coord(BaseCat, conn)`** (`category.rs`) — a deferred prop-ending coordination list (⟦·⟧ =
      `List ⟦BaseCat⟧`), the prop-side analogue of `cat_group`. **`coordinate_prop`** builds/extends it
      binarily (comma → neutral `conn_list`, `and`/`or` set/rebind the operator — the same
      `LIST_CONN`-neutral logic Step 5b added for NP groups); **`complete_coord`** folds the members
      with the operator (`op(op(m₀,m₁),…)`) — a **unary shift** in BOTH CKY paths (`UnaryKind::
      CoordComplete` packed; a composed-cell shift unpacked). The left-branching NF is enforced by
      `coordinate_prop` (right conjunct is never a list nor a completed `And`/`Or`).
    - **Retired**: the eager `coordinate_sem`; the Step-5b n-ary `parse_at_cap` fold; the `comma →
      unpacked` route in `parse_needs_unpacked`. Comma coordination is now **packed** (binary
      `Coordinate` edges + the `CoordComplete` unary shift — the packed hyperedge model expresses the
      list model directly), so the router `packing_router_decision_is_correct` re-asserts comma → packed.
    - **Verified**: kernel lib 1611, `closed_class` 126 (new `coordination_unpacked_via_list_completion`
      + `prop_coordination_builds_a_list_and_completes_by_folding`), clippy clean. Over 07-06 the refactor
      is behavior-preserving on every Step-5 case (`the MMR genes MSH2, MSH6, PMS2 or MLH1 …` CLOSED×168,
      bare comma-`or` CLOSED×4, felicity reject GAP) and **recovers** coverage the workaround dropped
      (adjective `colon, gastric and ovarian cancers` back to CLOSED×206 from ×54 — the unpacked routing
      had been more restrictive). The prep-object cap/beam residual is unchanged (still a mass-shim
      issue, not coordination).
- [ ] **Step 6 — RC-2: comparative `than` (2 units).** The `than`-clause complement (`greater/fewer X than
      Y`). Mirror the existing `cat_pp_than` argument-PP machinery for the `than`-phrase.
- [ ] **Step 7 — RC-3: `V X as Y` predicative small-clause (2 units).** `evaluate/identify X as Y` — the
      `as`-complement. Same shape as Step-2b's object+PP; likely folds into it (`((S\NP)/cat_as)/NP`).
- [ ] **Step 8 — RC-4: adjective + PP-complement (1 unit).** `concordant with X` — a predicative adjective
      subcategorizing for a PP. The adjective analog of Step-2's verb `cat_pp_arg`: `(S[adj]\NP)/cat_pp_arg`.
- [ ] **Step 9 — RC-5: linking-verb + predicate (1 unit).** `remain/stay/become X` taking an AP/NP
      predicate — a copula-class beyond `be`.
- [ ] **Step 10 — RC-7: copula kind on a compound subject (1 unit).** Make `are_kind` fire on a multiword
      compound bare-plural subject (`nucleotide repeat regions are microsatellites`) — a reshape edge case;
      the simple-subject path already works (`regions are microsatellites` CLOSED).
- [ ] **Step 11 — RC-8 + the `?` residual (≈4 units).** `hypothesize that … give rise to …` (clausal +
      multiword verb), the deep/compound object NPs (`… responses to immune checkpoint blockade`), and the
      un-probed verb+PP-or-object cases (2, 9, 17). Probe each to localize before fixing.
- [x] **Step 12 — Re-measure (DONE, `2026-07-06`).** `scripts/measure-parse-rate.sh --no-llm` over
      `wordnet-umls-all-2026-07-06` (cnl-v2, 74 min): 62 units → **ENCODED 0, AMBIG 50, GRAMMAR-GAP 12,
      MISSING 0** — 81% close (from 68% at Step-9). Step 5/5b/5c closed **8 gaps**. The 12 residual gaps are
      the phase-2 backlog in the roadmap header (RC-2 first). `missing-lexeme → 0` ✓ (phase 1 closed);
      grammar-gap → 0 is the remaining phase-2 gate. Re-run after each RC closes; the reranker pass is the
      phase-3 ambiguity metric, the `--no-llm` pass the does-it-parse gate.

Each step re-runs the measure over just its affected sentences (fast) before a full re-measure.

---

## 5. Caveats / notes

- **~~No doc glossary in this run.~~ RESOLVED (`2026-07-05`).** The harness now injects the Stage-A
  augmentation — abbreviation aliases **and** form/description-grounded OOV atoms — via the `LexicalIndex`
  document-augmentation overlay (`with_document_augmentation`), so the parser seeds them uncommitted over
  the DB head (§1a). As predicted, this closes the *lexical* blocker but does **not** rescue the
  grammar-gaps — they are verb-frame / construction gaps (§3), independent of the lexicon.
  - *Implementation note:* the overlay resolves each alias's cat/sem over the Arc chain (storage-
    independent), so it works over a DB-backed head where the value-index probe can't see uncommitted
    entries — closing the §7-2 "in-memory overlay over the persisted lexicon OOMs" gap without committing
    doc-scoped proposals to the store.
- **Ambiguity (0 encoded) and long-sentence perf are step 3**, deliberately excluded. Closing the gaps
  moves units *into* AMBIG; collapsing AMBIG→encoded is the next phase.
- **Grade of claims here:** the classification counts and the OOV list are **Derived** (the run). The
  verb+PP-frame *root cause* is a **Declared** hypothesis — Step 1 confirms it against the emitted cats
  before any fix lands.

---

## 6. Phase 3 — Ambiguity (the mass-shim over-generation)

**Witnessed (`2026-07-06` re-measure):** all 50 closing units are AMBIG, **none ENCODED**. Readings/unit
over the 50: min 4, **median 105, mean 125, max 256** — and 256 is exactly `DEFAULT_FOREST_CAP`, so the
top units are *capped* (true ambiguity is higher than measured).

**Root cause — the mass-shim over-generation.** The RC-1 fix (Step 4) marks a UMLS concept `mass` when its
preferred-name HEAD is uncountable (head-inheritance), so bare abbreviations of mass phenomena parse as
subjects (`MSI`). The head-inheritance is **loose**: it over-generates `mass` readings (e.g. `gene` picked
up a bogus `mass` from the junk atom `gENE` on "Gross Extranodal Extension", head "extension"). Every such
extra reading multiplies through the chart. This is the **binding constraint on phases 3 AND 4** (§7).

**Concrete tasks (the mass-shim precision follow-ups, tracked from Step 4).** **SUPERSEDED by the structural
fix — [d63-countability-from-subsumption.md](d63-countability-from-subsumption.md) (`2026-07-09`):** replace
the loose lexical head-inheritance with **countability by `is_subclass_of`** over the shared `lexicon:Entity`
lattice both importers already populate (UMLS mass-denoting TUIs; WN mass-denoting supersenses), with a
curated per-lemma **override** for grammatical-vs-ontological divergences (`furniture`). This eliminates the
head-string heuristic — and both patches below — by construction (`gene`'s TUI is a discrete gene type, so
no `gENE`→"extension" collision). The two patches are kept here only as the *heuristic* alternative:
1. **Strictly-uncountable-head test** — mark `mass` only when the head noun is uncountable in *all* its
   senses (not "some sense is uncountable"), killing the `extension`/`instability`-adjacent false positives.
2. **Acronym ↔ domain-word collision filter** (user-endorsed) — drop a `mass` (or any) reading for an
   acronym atom that collides with a primary concept of its own research domain (`gENE` = GENE).
3. Re-measure `--no-llm` (does the AMBIG×N median drop?) and with the **reranker** (`--features use-llm`,
   the phase-3 metric proper: AMBIG → single ENCODED per unit).

### 6a. The ambiguity is structural × sense — two distinct levers (Derived, `2026-07-08`)

Re-ran the chart-cell instrumentation over `wordnet-umls-all-2026-07-08` (§4b of
[d63-parsing-scale-and-pruning.md](d63-parsing-scale-and-pruning.md)). The readings-per-unit factor into a
**product of two independent sources**, each with its own lever:

- **Sense multiplicity** (×4–16 per skeleton) — each noun slot filled by a WordNet *or* UMLS sense, plus
  the mass-shim over-generation above. Lever: the **reranker/cap** (already built) + the mass-shim
  precision fixes (tasks 1–2).
- **Structural multiplicity** (2–36 bracketing / adjective-vs-compound category-choice skeletons per
  sentence) — the Catalan blow-up of the prenominal modifier stack (`attractive synthetic lethal targets`).
  Sense-ranking does **not** reduce it. Lever: a **nominal-modification normal form** — design note
  [d63-nominal-modification-normal-form.md](d63-nominal-modification-normal-form.md).

The two multiply (S5: 3 structural × 16 sense = 48), so **both levers are load-bearing** and a single clean
ENCODED reading needs both. The refined-noun `cat_n(Σ_)` shape dominates the saturating mid-chart cells
(32 of 173 non-leaf cells), so the structural lever is where the mid-chart population concentrates. The
`2026-07-06` "mass-shim is *the* root cause" framing is refined: the mass-shim is the sense-side
over-generation; the bracketing multiplicity is a **separate** structural source the mass-shim fixes don't
touch.

## 7. Phase 4 — Performance (parse-time under ambiguity)

**Witnessed (`2026-07-06`):** 62 units in **74 min**, with pathological outliers — a 14-token unit took
**930 s** (and still GAPPED), others 572 s / 286 s / 198 s. Parse time is **not** a function of position or
length: same-length units vary up to **1500×** (unit 38 @ 16 tok = 0.7 s vs unit 28 @ 16 tok = 198 s; late
units 48/50/53 = 0.1–0.6 s). The driver is **how many highly-ambiguous domain terms the sentence
contains** — the candidate-parse count explodes combinatorially and each candidate is kernel-felicity-
checked over the full lexicon chain. (unit 0's 27.9 s is cold-start warmup, not difficulty.)

**Concrete tasks:**
1. **Kill the ambiguity at source** — §6's mass-shim precision fixes collapse both the reading count and
   the parse time (they are the same root cause).
2. **Intermediate-cell beam** — the forest cap bounds only the TOP cell; a per-cell beam on the composed
   (non-leaf) cells bounds the blow-up (the `d63-parsing-scale-and-pruning.md` sub-project: adaptive
   supertagging + mid-chart felicity pruning, GH#97).
3. **Felicity-check cost** — profile whether the per-candidate kernel type-check over the full chain is the
   dominant term on the pathological units (cf. the earlier `axiom_env` / `build_axiom_env` full-scan
   findings); if so, index-drive or memoize it.
