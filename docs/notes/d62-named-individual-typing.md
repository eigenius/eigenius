# D62 — named individuals vs concept classes: typing domain symbols (genes) as proper nouns

*Design + grounding note. How a domain symbol (a gene like `WRN`) should be typed so it works as both
a **proper-noun NP** (`WRN is a vulnerability`) and a **prenominal modifier** (`the WRN gene`,
`KRAS mutations`). Grading: **Derived** = grounded in code / the UMLS 2026AA data / measured runs;
**Declared** = a design choice this note makes.*

## 0. The grammar is already ready — the gap is import typing (Derived)

A symbol typed as a **named individual** `cat_np(C, sg)` gets *both* readings for free (verified on the
small lexicon with `BRCA1 : cat_np(Gene, sg)` — all four parse):

| reading | example | mechanism |
|---|---|---|
| proper-noun NP (subj/obj) | `BRCA1 affects HeLa`, `HeLa affects BRCA1` | name → type-raise |
| prenominal modifier | `the BRCA1 gene affects HeLa` | named-entity compound `[cat_np][cat_n(C)] → Σx:C. compound(x, m)` (`parser.rs:373`, commented *"BRCA1 cell line"*) |

So nothing in the **grammar** is missing. The root cause is purely **import typing**: the UMLS
importer types *every* concept as a common noun `cat_n(umlscui:CUI, num_any)`
(`crates/eigenius-umls/src/convert.rs:140`). Hence at full-lexicon scale `WRN` is a *known* common
noun — it can't be a bare-singular subject, and `the WRN gene` would use the *kind*-compound, not the
named-entity one. (NCBI Gene already types genes as NP individuals — `lib.rs:22` — but it isn't
loaded; the 165 MB `--out-dir` partitioning gap.)

## 1. Does UMLS let us decide individual-vs-class systematically? Yes — a *triple* (Derived)

No single field suffices, but three fields the importer already parses (`rrf.rs`) do, together. The
`WRN` case proves the need for all three — the *same* string is a gene **and** a disease acronym:

| CUI | TUI (STN) | SAB | TTY | → |
|---|---|---|---|---|
| C1337007 (WRN **gene**) | T028 Gene or Genome (`A1.2.3.5`) | **HGNC** | ACR | **individual** `cat_np` |
| C0043119 (Werner **Syndrome**) | T047 Disease (`B2.2.1.2.1`) | OMIM | ACR | **class** `cat_n` |

1. **SAB = nomenclature authority** — strongest signal. **HGNC** exists to assign gene symbols; an
   HGNC atom *is* a named gene. In our level-0 subset: **44,052** of the **89,344** T028 gene CUIs
   carry an HGNC atom (each with `PT`+`ACR` symbol) — the human genome, which is what the WRN paper uses.
2. **TUI / STN = semantic category** — disambiguates gene (T028) vs disease (T047), and the STN tree
   number encodes the hierarchy: `A1…` Physical Object, `A2…` Conceptual Entity, `B…` Event.
3. **TTY = which string is the symbol** — `ACR`/`PT` is the rigid designator (`WRN` → the `cat_np`
   form); the descriptive name ("Werner syndrome RecQ like helicase") is a longer naming string.

**The honest caveat:** UMLS marks *everything* as a concept — there is no native individual flag. So
these are systematic *hooks*; the instance↔class boundary is **our modeling policy** on top of them.
The good news: it's a short, auditable rule, not per-concept guesswork.

### Why no single axis works (Derived — the survey)
- A blanket `A1`(physical)→individual **overreaches**: A1 is 71% of UMLS, dominated by organisms
  (Eukaryote 335k), chemicals/drugs (Organic Chemical 240k, Clinical Drug 209k), body parts
  (T023 "liver" — a common noun). Not uniformly proper nouns.
- Even a "named" TUI is heterogeneous: **T025 Cell** holds both `HeLa` (cell-line individual) and
  "neuron"/"T cell" (common-noun cell *types*). And there is **no cell-line authority** (CLO /
  Cellosaurus / CVCL / ATCC) in our subset — `HeLa` is just an `NCI/PT` term. So **cell-line-as-proper-
  noun is NOT systematically separable** in our data.

## 2. Scope for the WRN paper (Derived — the page + the measured OOV)

The WRN first page's named-individual needs are **exactly gene symbols, all HGNC-covered**:

`WRN, MLH1, MSH2, MSH6, PMS2, KRAS, BRAF, PARP1` — each with an HGNC `ACR` symbol. They unblock
`WRN is a synthetic lethal vulnerability`, `the helicase activity of WRN`, `KRAS mutations`,
`the MLH1 promoter`, `PARP-1 inhibitors` (both readings).

**The paper names no cell line** (no `HeLa`); cell lines are referred to *generically* (`cell lines`,
`MSI models`) — common nouns. **So the messy cell-line case is out of scope.**

**`MSI`/`MSS`/`MMR` are NOT OOV failures** (Derived — current-snapshot measurement). The page's full
OOV is **13 tokens**: `although, because, cas9, cas9-mediated, datasets, double-stranded, genome-scale,
hypermutable, hypermutations, msi-predominant, next-generation, pcr-based, recq`. None are the
abbreviations or genes. In fact **`WRN` and all gene symbols are *known*** — which is exactly why those
units come back **grammar-gap, not missing-lexeme**: known, but typed as common nouns. Breakdown of
the 13: subordinators `because`/`although` (2), genuine domain `cas9`/`recq` (2), S0 hyphenated
compounds (6: `cas9-mediated`, `double-stranded`, `genome-scale`, `next-generation`, `pcr-based`,
`msi-predominant`), morphology (3: `datasets`, `hypermutable`, `hypermutations`).

## 3. The policy (Declared)

A concept is a **named individual** (`cat_np`, both readings) iff, in priority order:
1. it has an atom from a **named-entity nomenclature SAB** (start with **HGNC** → genes), using its
   **symbol TTY** (`ACR`/`PT`) string as the proper-noun form; **else**
2. *(later)* it is in a whitelisted named-entity TUI *and* carries a symbol-like TTY (a narrow,
   deliberate set — not all of `A1`); **else**
3. **default → concept class** (`cat_n`, unchanged).

This keeps the boundary a short, citable table (SAB allow-list + TUI whitelist + TTY symbol-set).

## 4. Sense, not OOV: in-document abbreviation resolution (Declared — adjacent improvement)

`MMR` is *known* but UMLS has many `MMR` concepts (mismatch-repair vs measles-mumps-rubella vaccine…);
nothing guarantees the right binding. Meanwhile S0's `strip_bracketed_asides` **discards** the text's
own `microsatellite instability (MSI)` / `DNA mismatch repair (MMR)` apposition — i.e. we throw away
the author's authoritative definition and rely on ambiguous lexicon lookup. The faithful fix is
**in-document abbreviation resolution**: parse `full term (ABBR)`, register `ABBR := that concept`
document-locally, bind later bare uses to it. This is a *sense/faithfulness* improvement (grounded in
the document, D61), **not** an OOV blocker — a separate track from the typing work here.

## 5. Implementation — DONE + validated

1. **Importer (`crates/eigenius-umls`).** `ConceptBuilder` tracks the per-CUI symbol from
   `NAMED_INDIVIDUAL_SABS` atoms (HGNC `ACR`>`PT`) → `Concept.symbol`. A named individual emits as an
   **instance** of its primary TUI class (`resource umlscui:CUI : umlssty:T028`) with `cat_np(umlssty:T028,
   sg)` entries (`sem =` the instance); concept classes stay `cat_n`. Also fixed a latent bug: a
   `resource` body needs qualified `core:description` (bare `description` is a *class*-item keyword).
   TDD: 9 builder + 2 convert unit tests; real-data validate of 3k genes → 27,557 `cat_np` entries
   felicity-gate clean.
2. **Reseed** (`scripts/reseed-lexicon-db.sh`, WordNet `--all` + UMLS WRN-TUI subset incl. T028) →
   634 MB snapshot `wordnet-umls-2026-06-29`; both chains loaded clean (the gene `cat_np` entries
   validated at load).
3. **Validated** over the reseeded store (`diagnose_grammar_gap_fragments`):

   | fragment | before | after |
   |---|---|---|
   | `WRN is a vulnerability` | grammar-gap | **CLOSED×2** |
   | `WRN is a vulnerability and a target` | grammar-gap | **CLOSED×2** |
   | `WRN is a vulnerability for MSI cancers` | grammar-gap | **CLOSED×8** |

   WRN now parses as a proper-noun subject (and `the WRN gene` as a modifier — verified on the small
   lexicon). Full WRN page: 0 panics; the residual is an OOV tail, **inflated by the TUI-*subset*
   reseed** (it omits `microsatellite`/`biomarker`/`crispr`/`germline`/`vitro`/`vivo` — TUIs outside
   the 8) — use `--umls-all` for a clean full-page number.

### 5a. Latent readback panic the gene-typing exposed — FIXED

Making genes `cat_np` (resource sems) surfaced a pre-existing composition bug: a **proper-noun
(resource) subject + an adjective-refined predicate nominal** (`HeLa is a large gene` /
`WRN is a synthetic lethal vulnerability`) built an ill-formed term that applied the **subject** as a
function → `readback_val` `NotAFunction` panic. Root cause: the refined-noun **Fst-projection** case
(`parser.rs`, D63 §8.5 3b) — correct only for a **GQ** determiner (2nd arg = a restrictor predicate
`V` over the noun type) — misfired for the **predicate-nominal** `a_pred` (`λT.λs. is_a(s,T)`), whose
2nd arg is the *subject* and whose body (`S[adj]\NP(Entity)`) does **not** mention `T`. Fix: **gate the
Fst case on `tvar ∈ body`** (the determiner actually binds a predicate over the noun type); else use
the simple case (`a_pred(Σ) = λs. is_a(s, Σ)`). The `readback_val` `.expect()` invariant is kept
(readback never sees an un-type-checked term *because we no longer build one*); 83 determiner + full
kernel tests green, with a regression test `predicate_nominal_over_refined_noun_parses`.

Out of scope here: cell lines (no authority — needs Cellosaurus or a heuristic), organisms/proteins,
the in-document abbreviation track (§4), and stacked-adjective predicate nominals (`synthetic lethal`
— a clean grammar-gap now, not a crash).
