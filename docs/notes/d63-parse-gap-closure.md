# D63 — Parse-gap closure for the test document (full-lexicon baseline + plan)

**Status:** measured baseline + ordered closure plan. Goal (the user's step 2): make the test document
**parse completely** — every unit produces ≥1 parse — over the full WordNet+UMLS lexicon. Ambiguity
(collapsing the many readings to one) and long-sentence perf are the *next* phase (step 3), not this one.

Sequence this note sits in: **full-lexicon run (done, below) → close lexicon+grammar gaps (this note) →
address ambiguity + holes → close grading-phase gaps.**

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
  Parses the raw page over the base lexicon — **no Stage-A doc glossary injected** (see §5 caveat).

**Result line (Derived — verbatim from the run):**

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

Two meta-findings (Derived), both **out of scope here** (step 3):
- **0 encoded** — every parse is ambiguous (AMBIG ×8 to ×64). Sense-crowding; the reranker exists to
  collapse it.
- **Long sentences cost 3–5 min** — Lever B beam dropping *millions* of chart items on the 16–21-tok
  units (e.g. 325 s on unit 47). A perf concern; nothing hit the hard scale bound.

---

## 2. Gap class 1 — OOV (small: 4 tokens, all domain-lexicon)

6 units, each blocked by exactly 1 OOV; 4 distinct tokens:

| token | units | shape | fix |
|---|---|---|---|
| `double-stranded` | 15 | hyphenated adjective | domain-lexicon adjective entry |
| `hypermutable` | 21 | `hyper-` + adjective | domain-lexicon adjective entry |
| `pcr-based` | 45, 49 | `X-based` denominal adjective | domain-lexicon adjective entry |
| `recq` | 48, 50 | gene-family name | domain-lexicon named entity (`cat_np`) |

---

## 3. Gap class 2 — grammar-gap (17), and it is *mostly lexical*

**The dominant pattern is missing verb + PP-complement frames**, established by a controlled contrast in
the data (Derived):

> *"MSI **is observed in** colorectal, endometrial, gastric and ovarian cancers"* → **parses (AMBIG)**
> *"MSI **occurs in** colon, gastric, endometrial and ovarian cancers"* → **grammar-gap**

Same coordination, different verb ⇒ the coordination isn't the blocker; the **verb's PP-complement
subcategorization frame is**. `observe/limit/depend` compose with their PP; `occur/arise/contribute/
result/respond/associate/query/compare` do not. This matches the standing finding that the parser is
~grammar-complete and the residual is lexical.

The 17 grammar-gap sentences, bucketed (a sentence can carry >1 blocker):

**A. Verb + PP-complement frame (~10) — highest leverage:**
- `query … in` — "We queried dependencies in cancers with MSI."
- `result from` — "MSI results from deficient DNA mismatch repair."
- `contribute to` — "MSI contributes to several cancers."
- `occur in` — "MSI occurs in colon, gastric, endometrial and ovarian cancers."
- `arise from` — "MSI can arise from Lynch syndrome." / "Somatic MMR inactivation … arises from hypermethylation of the MLH1 promoter."
- `associate with` — "MSI is associated with notable responses to immune checkpoint blockade."
- `respond to` — "Some cancers do not respond to immune checkpoint blockade."
- `compare to` — "The MSI relationship compared favourably to other strong biomarkers for vulnerabilities." / "… compared to MSS cell lines" (unit 47).

**B. Comparative `than` (2):** "showed **greater** dependence … **than** their MSS counterparts" /
"contained **fewer** deletion mutations … **than** typical lineages". (`less dependent on` *alone* parses,
so the `than`-clause is the blocker.)

**C. `V X as Y` predicative (2):** "evaluated MSI **as a biomarker**" / "identified WRN **as** the top
preferential dependency".

**D. Coordination / apposition in context (3):** apposition "the MMR genes **MSH2, MSH6, PMS2 or MLH1**";
"**Some** MSI lines **and some** MSS lines … were represented by …"; "require specific lineages **or** a
stronger mutation phenotype". (Plain adjective coordination parses — unit 53 — so these gap on the
apposition / NP-coordination / passive context, not lists per se.)

**E. Copula compound kind (1):** "Nucleotide repeat regions **are** microsatellites" — the reshape's
`are_kind` (kind–kind subsumption) path not firing on a **3-word compound** subject.

**F. Named disease (1–2):** "**Lynch syndrome**" — named-entity handling (also appears in A/D sentences).

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
        under the operational reranked config. *Implication: the honest metric for this fix is the reranked
        pass, and sense-crowding is now the pressing blocker (it masks the fix on the raw run).*
  - *Looseness (stage-1):* WordNet frames don't encode *which* preposition, so `cat_pp_arg` accepts any PP
    (`contributes in cancers` would also parse) — verb-specific but prep-generic; specific-prep is later.
- [ ] **Step 3 — Add the 4 OOV** (`double-stranded`, `hypermutable`, `pcr-based` adjectives; `recq`
      named entity) as domain-lexicon entries. Closes all 6 missing-lexeme units.
- [ ] **Step 4 — Comparative `than`** (bucket B) — the `than`-clause construction. 2 units.
- [ ] **Step 5 — `V X as Y` predicative** (bucket C) — the "as a biomarker" / "identified as" small
      clause. 2 units.
- [ ] **Step 6 — Coordination / apposition in context** (bucket D) — noun-noun apposition
      ("the N genes <list>"), NP coordination under a quantifier ("some X and some Y"), object `or`. 3 units.
- [ ] **Step 7 — Copula compound kind** (bucket E) — make `are_kind` fire on a compound bare-plural
      subject. 1 unit (a reshape edge case — `Σ`-refined subject on the kind–kind path).
- [ ] **Step 8 — Named disease** (bucket F) — `cat_np` injection for named entities ("Lynch syndrome").
- [ ] **Step 9 — Re-measure** `scripts/measure-parse-rate.sh --no-llm`. **Gate:** grammar-gap +
      missing-lexeme → 0 (every unit parses, at least AMBIG). That is "the test document parses completely."

Each step re-runs the measure over just its affected sentences (fast) before the full re-measure at Step 9.

---

## 5. Caveats / notes

- **No doc glossary in this run.** The harness parses the raw page over the base lexicon; the Stage-A
  document glossary (abbreviation aliases) is *not* injected. This does **not** rescue the grammar-gaps —
  they are verb-frame / construction gaps, and `MSI` already parses as a modifier/subject in the AMBIG
  units. The glossary matters for OOV *abbreviations*; the 4 OOV here are adjectives + a gene name, not
  abbreviations.
- **Ambiguity (0 encoded) and long-sentence perf are step 3**, deliberately excluded. Closing the gaps
  moves units *into* AMBIG; collapsing AMBIG→encoded is the next phase.
- **Grade of claims here:** the classification counts and the OOV list are **Derived** (the run). The
  verb+PP-frame *root cause* is a **Declared** hypothesis — Step 1 confirms it against the emitted cats
  before any fix lands.
