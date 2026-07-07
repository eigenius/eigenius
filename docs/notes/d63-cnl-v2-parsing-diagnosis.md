# D63 — CNL-v2 parsing diagnosis (consolidated)

**Status:** Derived (all figures witnessed by the runs cited). Session 2026-07-01/02.
**Target corpus:** `references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v2.txt` — the
controlled-language rewrite of the WRN-helicase Nature first page (62 segmented units).
**Harness:** `crates/eigenius-wordnet/tests/db_backed_encoding.rs::wrn_first_page_over_full_lexicon`
(+ the `probe_*` diagnostics added this session), over the full WordNet+UMLS lexicon, with the live
Anthropic sense reranker (`--features allms`). Reproduce via `scripts/measure-parse-rate.sh`.

---

## 0. Headline

Running the CNL-v2 page end-to-end and then **probing every grammar-gap sentence to its cause**
overturned the going-in assumption. The distilled result:

1. **PP-attachment is NOT a lever for this corpus — 0 of 19 grammar-gaps.** (It was the top lever on
   the *raw* prose's prep-heavy piles, blueprint §10c — a different target.)
2. **The single dominant real blocker is bare domain abbreviations used as argument NPs** (`MSI`
   subject/object) — a **lexicon/discourse** issue (abbreviation-definition handling), ~8 of 19 gaps.
3. **The "19 grammar-gaps" overstates the real grammar frontier.** At least 2 (likely more) are
   **beam / lexicon-crowding artifacts** — sentences that parse on the subset lexicon and gap only
   under full-UMLS sense density. Genuine grammar gaps are **~4 narrow constructions**.
4. **Full UMLS is the right lexicon fix** — it recovered exactly the OOV lemmas predicted and lifted
   any-parse 48% → 60% — but it *also* introduces crowding gaps on complex sentences (a tuning
   tension, not a coverage failure).

The recurring methodological lesson: **every Declared "this is the gap" categorization shrank or
flipped under a minimal-pair probe.** Prep-verb subcategorization went to 0; comparatives from ~4 to
~2; ~8 residual "grammar" gaps turned out to be non-grammar. Witness-before-conclude paid rent
repeatedly.

---

## 1. Measurements

### 1a. Subset vs full-UMLS (CNL-v2, 62 units, live LLM)

| outcome | subset (WRN-TUI UMLS) | full-UMLS (`--umls-all`) | Δ |
|---|---|---|---|
| ENCODED (single closed parse) | 0 | 0 | — |
| AMBIG (multiple closed) | 4 | 2 | −2 |
| OPEN (parsed, referent holes) | 26 | **35** | +9 |
| MISSING (OOV) | 15 | **6** | **−9** |
| GRAMMAR-GAP (known, no parse) | 17 | **19** | +2 |
| SCALE-BOUND | 0 | 0 | — |
| **any-parse** (enc+amb+open) | 30 (**48%**) | 37 (**60%**) | **+12 pts** |

Snapshots (both preserved for A/B): subset `db-snapshot/wordnet-umls-2026-07-02` (697 MB); full
`db-snapshot/wordnet-umls-all-2026-07-02` (2.1 GB). Reseed via `scripts/reseed-lexicon-db.sh`
(`--umls-all` for the full one).

**Reading:** the MISSING → OPEN swing (−9 / +9) is the core result — recovering the domain nouns lets
those sentences *parse* (into open, awaiting anaphora), not hit a fresh wall. So lexicon coverage was
a **genuine blocker, not a surface symptom**. OPEN (35) is the biggest bucket and is a *post-parse*
concern (anaphora / referent resolution, D64) — set aside for the parsing-frontier analysis.

### 1b. The OOV bucket (coverage analysis, witnessed vs MRCONSO/MRSTY + `convert.rs`)

The subset's 15 MISSING units (12 distinct OOV tokens) split cleanly, **predicted before running full
UMLS and confirmed exactly**:

- **Recovered by full UMLS (6 lemmas)** — exist as single-token concepts under TUIs the WRN-subset
  didn't load: `biomarker` (T201 Clinical Attribute), `microsatellite` (T114/T123), `crispr` (T114),
  `germline` (T033 Finding), `hypermethylation` (T045 Genetic Function), `phenotyping` (T169/T059).
  The importer roots **every** semantic type at `lexicon:Entity` and emits every concept as a
  `cat_n(umlscui:C…)` common noun regardless of TUI (`convert.rs:102`, `:162–172`) — so `--umls-all`
  makes them usable Entity-rooted nouns. Full-UMLS residual OOV = exactly these gone.
- **Not a UMLS lemma (4 tokens)** — no single-token concept exists: `recq` (only multi-word "RecQ
  Family of DNA Helicase"), `double-stranded` / `pcr-based` (adjectival hyphenated compounds),
  `hypermutable` (adjectival `-able` derivation). These need named-individual injection (recq) or
  adjective/morphology handling — **full UMLS does not help them**, and indeed they are the entire
  full-UMLS residual OOV.

---

## 2. The 19 grammar-gaps, diagnosed to cause

Every gap sentence was bisected with minimal pairs over clean known vocab (so a gap is the construct,
not a confound). Final Derived breakdown:

| cause | count | nature | fix locus |
|---|---|---|---|
| **bare abbreviation as an argument** (subj / obj / prep-obj) | **~8** | lexicon/discourse | `Long(ABBR)` handling |
| **lexicon-crowding beam artifacts** | **~2+** (2 confirmed; likely more of the untested 5) | tuning | beam-widen ceiling |
| **genuine narrow grammar gaps** | **~4** | grammar | `than NP` comparative ×2, compound-prep-object ×1, cross-type coordination ×1 |
| **PP-attachment** | **0** | — | — |

Numbers are approximate because a few sentences (4, 8, 9, 14, 16) contain full-UMLS-only tokens and
weren't re-probed on the subset; by extrapolation they are a mix of crowding + abbreviation-in-other-
positions.

---

## 3. Probe-by-probe (what each refuted / confirmed)

Each is an `#[ignore]`d diagnostic left in `db_backed_encoding.rs` (re-runnable, cap-only).

### 3a. `probe_prep_verb_gap` — REFUTED the going-in #1 hypothesis
Hypothesis (from `convert.rs::classify` dropping the PP on oblique frames 4/13/22 — a documented
stage-1 loss): prep-verbs (`result from`, `respond to`, `contribute to`, `arise from`) can't license
their PP → gap. **Refuted:** all prep-verb sentences PARSE (`OPEN×4–8`) — the VP-adjunct PP rule
composes the preposition regardless of the dropped complement. The dropped PP-complement is real but
**not** the blocker.

Then the bisection found the actual blocker:

| probe | result | reading |
|---|---|---|
| `cancers result from deficient DNA mismatch repair` (bare-plural subj) | OPEN×233 | ✓ |
| `cancers can arise from mutations` (modal) | OPEN | ✓ |
| `cancers do not respond to genes` (negation) | OPEN | ✓ |
| `genes contribute to several cancers` (determiner) | OPEN | ✓ |
| **`MSI` results from mutations / causes cancers / is a disease** | **GAP** | `MSI`-as-subject is the blocker |

Confirmation: `the MSI causes cancers` PARSES → `MSI` is a `cat_n` common noun needing a determiner.
`WRN is a gene` PARSES *bare* → `WRN` is a `cat_np` named individual. `HeLa is a gene` bare PARSES
(positive control). So the split is category, not knownness (both have `has_token = true`).

### 3b. `probe_comparatives` — most comparatives already parse
| form | example | result |
|---|---|---|
| synthetic `-er` + than | `genes are larger than cells` | OPEN ✓ |
| attributive comparative (no than) | `a stronger phenotype`, `greater dependence` | OPEN ✓ |
| comparative verb | `compared favourably to` / `compared to` | OPEN ✓ |
| periphrastic `more ADJ` + than | `genes are more essential than cells` | **GAP** |
| phrasal `than NP` (quantity) | `fewer mutations than genes`, `greater dependence than genes` | **GAP** |

Only the **`than NP` quantity comparative** actually bites the corpus (sents 17, 18). Comparatives are
a ~2-gap lever, **not ~4** — the `compared favourably to` and `a stronger phenotype` gap sentences fail
for *other* reasons (the probe ruled the comparative out).

### 3c. `probe_gap_tail` — almost everything parses in isolation
| group | example | result |
|---|---|---|
| **abbreviation as MODIFIER** | `MSI cells contain genes`, `WRN genes cause cancers`, `MMR mutations cause cancers` | **OPEN ✓** |
| `as`-predicative | `cells evaluated genes as targets` | OPEN ✓ |
| plural copula predicate-nominal | `regions are genes` | CLOSED ✓ |
| PP-stack | `… in cancers with mutations` | OPEN ✓ |
| numeral + adj + N-N-N | `two independent cancer dependency targets` | OPEN/CLOSED ✓ |
| modal + `or`-coordination | `genes may require cells or mutations` | OPEN ✓ |
| adj-subject + prep-verb / `that … essential in` | | OPEN ✓ |
| **compound as PREP-object** | `respond to immune checkpoint blockade` | **GAP** |

Load-bearing: **abbreviation as a *modifier* parses** (`MSI cells`, `WRN dependency`) — so the
abbreviation problem is *argument-position-specific* (a bare singular `cat_n` can't be an argument), not
general. This kept the abbreviation lever from ballooning, and left a genuinely heterogeneous tail with
one real new gap: a **compound noun as a prep-object** (sent 11).

### 3d. `probe_beam_crowding` — the residual tail is mostly artifacts
Actual residual sentences, verbatim, on the **subset** at default beam (64→512) and a wide fixed beam
(2048), vs their known full-UMLS GAP:

| sentence | subset @64→512 | subset @2048 | full-UMLS | verdict |
|---|---|---|---|---|
| `We found that WRN was selectively essential in MSI models.` | **OPEN×62** | OPEN×256 | GAP | **crowding artifact** |
| `We analysed two independent cancer dependency data sets.` | **CLOSED×12** | CLOSED×72 | GAP | **crowding artifact** |
| `WRN dependency may require specific lineages or a stronger mutation phenotype.` | GAP | GAP | GAP | **genuine grammar gap** |

Two of three parse on the subset → their full-UMLS GAP was **lexicon-crowding** (extra senses
over-crowd the beam; the 512 widen ceiling isn't enough), **not grammar**. The third gaps everywhere
(even beam 2048) → a real gap: **cross-type object coordination** — `[specific lineages]` (bare plural)
`or` `[a stronger mutation phenotype]` (determined singular); the `_obj` determiners bake NP shape into
the object category, so mismatched conjuncts don't share a category (the known cross-type-coordination
gap, GH#93 / D62 §2 #8 lineage).

---

## 4. Mechanism deep-dives

### 4a. Why `MSI` gaps but `WRN` doesn't — the UMLS root
`MSI` and `WRN` are both known; the difference is `cat_np` (named individual) vs `cat_n` (common noun),
which falls out of UMLS semantic typing via the importer's gene-symbol → named-individual detection:

- **`WRN`** = 3 concepts: **WRN gene** (C1337007, T028 Gene or Genome), WRN protein (C0388246, T116/T126),
  Werner Syndrome (C0043119, T047). The *gene* concept is a gene symbol → **named individual (`cat_np`)**
  → bare `WRN` works.
- **`MSI`** = **Microsatellite Instability** (C0920269, **T049 Cell or Molecular Dysfunction**) — a
  *dysfunction class*, no gene symbol → **common noun (`cat_n`)** → bare `MSI` can't be an argument.
  (Plus a junk collision: C5420097 "AML Myeloid Sarcoma Involvement Table", T170, same string.)

So UMLS models MSI *correctly for UMLS* (a dysfunction kind) but that is **wrong for how the paper uses
it** (a referring named entity, the paper's topic). Relying on the importer to reclassify MSI is
fragile (dysfunction typing + string ambiguity). The reliable signal is the document's own definition.

### 4b. The definition is present in the corpus but lost before the parser
- The **original** defines it: `…cancers with microsatellite instability (MSI), which results from…`
  (`first-page-cleaned.txt`).
- The **CNL-v2 rewrite dropped it** — "microsatellite instability" appears **0 times**; MSI is used cold
  from sentence 1.
- Even in the original, the tokenizer's `strip_bracketed_asides` (`lookup.rs:140–148`) **drops the
  `(MSI)` parenthetical** as a gloss — so the `microsatellite instability ↔ MSI` binding is never made;
  later bare `MSI` falls through to the UMLS `cat_n` entry.

Hence the fix is **document-local abbreviation handling**, three interacting parts: (1) detect the
`Long Form (ABBR)` pattern and inject a document-scoped named individual (alias for the long-form
concept); (2) **don't strip a *definitional* parenthetical** (extract before the drop); (3) the CNL
rewrite must preserve the definition (or (1)+(2) handle it on the original).

### 4c. Full UMLS: coverage win with a crowding tax
`--umls-all` recovered the 6 predicted OOV lemmas and lifted any-parse +12 pts — a real coverage win.
But it *also* regressed complex sentences that parsed on the subset (sents 3, 12), because the extra
sense density over-crowds the per-cell beam (widen ceiling 512). Net positive, but it exposes a
scaling tension: **more lexicon coverage costs beam headroom.** The knob is
`CELL_BEAM_WIDEN_MAX`/`SENSE_CAP` (GH#97 Lever A/B) — consistent with the lexicon-as-scaling-stress-test
philosophy.

---

## 5. Refuted / deprioritized (documented so we don't re-chase)

- **PP-attachment control** — the going-in top lever (from the raw-prose piles, blueprint §10c) — is
  **0 of 19 gaps on CNL-v2.** Deprioritized for this target. (Still plausibly relevant to the *raw*
  paper, a different corpus — that claim is untouched.) The scoping note
  `d63-pp-attachment-control-scoping.md` stands but is not the CNL-v2 lever.
- **Prep-verb subcategorization** — refuted (§3a): prep-verbs parse via VP-adjunct.
- **Comparatives as a big grammar lever** — mostly parse; only the `than NP` quantity form (~2, §3b).
- **Modals, `do`-support negation, determiners, compound objects, `as`-predicative, plural copula,
  N-N-N compounds, abbreviation-as-modifier** — all parse in isolation (§3a/§3c); none is a blocker.

---

## 6. Prioritized levers (for the CNL-v2 target)

1. **Abbreviation-definition handling** (~8 gaps, #1) — `Long Form (ABBR)` extraction + a
   `strip_bracketed_asides` exemption for definitional parentheticals + document-local named-individual
   injection. Lexicon/discourse layer; generalizes to every real paper (MMR, PARP, MSS, …).
2. **Beam-widen ceiling / crowding tuning** (~2+ gaps, cheap) — raise `CELL_BEAM_WIDEN_MAX` (currently
   512) and re-measure; recovers the crowding artifacts that full UMLS introduced.
3. **~4 narrow genuine grammar gaps** — `than NP` quantity comparative (×2), compound-noun prep-object
   (×1), cross-type object coordination (×1, GH#93). Small, deferrable.
4. **Anaphora / referent resolution** (OPEN = 35, the biggest bucket) — *post-parse*, separate track
   (D64). Converts OPEN → ENCODED; not a parsing lever.

**Not** on the list: PP-attachment (0 gaps here), prep-verb subcategorization (refuted), a general
comparative rule (mostly already works).

---

## 7. Reproducibility

- **Measure:** `scripts/measure-parse-rate.sh [--page cnl-v2|original|cnl] [--no-llm] [--snapshot DIR]`
  (autodetects the newest snapshot; requires `ANTHROPIC_API_KEY` for the live reranker).
- **Reseed:** `scripts/reseed-lexicon-db.sh [--umls-all] [--snapshot-dir DIR]` after any bootstrap edit.
- **Probes** (in `crates/eigenius-wordnet/tests/db_backed_encoding.rs`, `#[ignore]`d, run with
  `EIGENIUS_DB_SNAPSHOT=<snap> cargo test -p eigenius-wordnet --test db_backed_encoding <name>
  -- --ignored --nocapture`): `probe_prep_verb_gap`, `probe_comparatives`, `probe_gap_tail`,
  `probe_beam_crowding`.
