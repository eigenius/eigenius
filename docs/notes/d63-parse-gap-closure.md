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

- [ ] **Step 1 — Diagnose the verb+PP-frame root cause** *before* patching. Is it the WordNet importer
      emitting the wrong `cat` for verbs that take a PP argument (a *systematic* fix), or a grammar-rule
      gap? Entry point: the `observe in` (parses) vs `occur in` (gaps) contrast — compare the two verbs'
      emitted `LexicalEntry` cats. **This determines the shape of the whole step** (import-level vs
      grammar-level vs per-verb). *Hypothesis (to confirm): the importer gives these verbs an
      intransitive/transitive cat but no `(S\NP)/PP` frame.*
- [ ] **Step 2 — Close bucket A (verb+PP frames)** per the Step-1 diagnosis. Systematic if it's the
      import; closes ~10 of 17. Re-measure the affected units.
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
