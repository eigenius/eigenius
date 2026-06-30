# Eigenius Controlled English — a style guide for parser-faithful scientific prose

*D62 experiment, 2026-06-29. A controlled natural language (CNL) for writing factual scientific
claims that the Eigenius DCG/CCG parser fully covers, so the encoding captures the **claim** (a
kernel-checked `Prop`), not an approximation. Grounded in the parser's *actual* capabilities as built
through D62/D63 (not aspiration). The companion experiment rewrites the WRN first page into this style
and measures parsing coverage (`first-page-cnl.txt`).*

## Purpose & posture

The parser is the oracle: a sentence either composes into a kernel-checked typed tree or it does not.
Rather than bend the grammar to arbitrary journal prose (long, compound, statistic-laden), **write the
science in the subset the parser covers**. This matches the encoding objective — we want the
*load-bearing factual claims* as checkable `Prop`s; rhetorical packaging, inline statistics, and
citations are out of the claim by design (D62 S0 routes them out).

Two rules sit above everything else:

- **(R1) One claim per sentence.** Almost every grammar gap below is dissolved by splitting a compound
  journal sentence into several short factual ones.
- **(R2) Faithfulness over parseability — never drop a *qualifier* to make a sentence parse.** A
  simplification may drop *data* (numbers, citations, figure refs — out of the claim by design), but it
  **may not** drop a word that changes the claim's **strength, scope, or modality**: modals
  (`can`/`may`), scalar/comparative adverbs (`preferentially`/`selectively`/`typically`/`highly`),
  scope restrictions (`the four RecQ helicases`, not `the helicases`), or severity/type specificity
  (`double-stranded`). If keeping a qualifier means the sentence does not yet parse, **keep it anyway
  and record the gap** — a faithful un-parsed claim is a tracked to-do; a parsed distorted claim is a
  silent error (the D61 faithfulness gap). See the audit + rule at the end of this note for why.

## DO — constructions the parser covers

1. **Subject–verb–object, one clause.** `WRN is essential in MSI models.` `Depletion of WRN promotes
   apoptosis.` Present tense (`affects`/`affect`) or simple past (`affected`, `was`/`were`).
2. **Predicate nominals & adjectives.** `WRN is a vulnerability.` `WRN is a drug target.` `The
   dependency is selective.` Copula present/past: `is`/`are`/`was`/`were`.
3. **Determiners.** `a`/`an`/`the`/`this`/`that`/`these`/`those`/`every`/`each`/`all`/`some`/`no`, and
   the cardinals `two`…`ten`. Bare plurals are fine (`Cancers exhibit defects.`) — they parse with a
   deferred quantifier (an *open* parse, which is acceptable).
4. **Coordination.** `and`/`or`; comma lists `X, Y and Z`; sentence-level `S but S`. Contrastive
   `requires A but not B` **when A and B are the same kind of thing** (e.g. two activities).
5. **Adjectives & compounds.** Stacked attributive adjectives (`a synthetic lethal vulnerability`);
   noun–noun compounds (`cancer models`, `cell line`, `MSI cancer models`).
6. **Prepositional phrases.** `of`/`in`/`for`/`with`/`on`/`from`/`within`/`between`, as noun
   post-modifiers (`a biomarker of dependency`) and verb adjuncts (`essential in MSI models`). The
   object may be a determined NP (`within a gene`, `for tumours`).
7. **Relative clauses.** Restrictive `the gene that affects X` / `which affects X`; non-restrictive
   `WRN, which encodes a helicase, is essential.`
8. **Passive.** `WRN was depleted.` `Apoptosis was promoted by depletion.`
9. **Negation.** `WRN does not affect MSS models.` `The activity is not essential.`
10. **Clausal complements (report verbs).** `These findings show that WRN is a vulnerability.`
11. **Transitional adverbs** (sentence-initial): `Thus,` `Therefore,` `Hence,` `Moreover,`
    `Similarly,` `Notably,` — transparent (they don't change the claim).
12. **Light verbs** that exist in the lexicon, e.g. `gives rise to`.

## DON'T — and how to rewrite it

| Avoid (journal style) | Why | Rewrite recipe |
|---|---|---|
| **Inline numbers / statistics** (`n = 37`, `P = 4.2 × 10⁻¹³`, `51 cell lines`, `0.56-fold`, `15%`) | The parser routes non-prose out; numbers are **dropped**, so a numeric claim is lost. | State the **qualitative** claim; put the statistic elsewhere (a separate D52 record). `… showed greater dependence …` not `(n=37; P=…)`. |
| **Parenthetical asides / inline abbreviations** (`(MSI)`, `(PARP-1)`, `(Fig. 1a)`) | Asides are dropped; the parenthetical can't be a claim. | Introduce an abbreviation in its **own** sentence, or just use one form consistently. Drop figure/citation refs. |
| **Em-dash appositives** (`—an interaction…—`) | Not covered; the dash content is dropped. | Split into separate sentences: `Synthetic lethality is an interaction between two genetic events. …` |
| **Long multi-clause sentences** (relative + subordinate + parenthetical stacked) | Each clause must compose; one gap kills the whole, and long units hit the beam. | **One claim per sentence.** |
| **`because` / `although` subordinate clauses** | Not in the lexicon (OOV); subordinators unbuilt. | Split + use a transitional: `…. Therefore ….` Drop concessive `although` or restate as two facts. |
| **Cross-type `but not`** (`required the helicase activity … but not its exonuclease activity` — different kinds) | The two objects must be the same category. | Split: `MSI models required the helicase activity of WRN. MSI models did not require the exonuclease activity of WRN.` |
| **Deeply-embedded / determined-subject pied-piping** (`the way in which the co-occurrence leads…`) | Only simple/name-subject pied-piping is covered. | Rephrase as a separate clause: `The co-occurrence leads to cell death.` |
| **Hyphenated compound terms** (`CRISPR–Cas9-mediated`, `double-stranded`, `genome-scale`) | Split oddly / OOV. | Use spaced or simpler forms the lexicon knows, or rephrase. |
| **Possessive ellipsis / heavy gapping**, fronted reduced clauses with complex complements | Limited; gapping beyond same-type `but not` isn't covered. | Use an explicit subject and a full verb in each clause. |
| **`and/or`** | Not a token; collapsing it to `and` overstates (requires *both*). | Write **`or`** — `logic:Or` is **inclusive** (true if either or both), which is exactly what `and/or` means. (Faithfulness rule, not just style — `and/or → and` is a meaning change; `and/or → or` is meaning-preserving.) |

## Vocabulary note (orthogonal to style)

Style ≠ vocabulary. Domain terms the lexicon doesn't know (`cas9`, `recq`, novel hyphenations) are
**OOV** regardless of style; the measurement reports OOV separately. Where a known synonym exists,
prefer it; otherwise keep the domain term and accept the OOV (a vocabulary-import question, not a
style one). Gene/entity symbols (`WRN`, `MSH2`) resolve as named individuals where the UMLS/HGNC
import provides them.

## Worked example (one WRN sentence)

**Original (journal):** *"MSI cancer models required the helicase activity of WRN, but not its
exonuclease activity."*

**Controlled:**
> MSI cancer models required the helicase activity of WRN.
> MSI cancer models did not require the exonuclease activity of WRN.

Two same-shape SVO clauses; the contrast is preserved as an explicit negation; both compose.

## Success criterion

A passage is "parser-faithful" when every sentence yields a **closed or open** kernel-checked parse
(no GRAMMAR-GAP), and the set of parses captures the passage's factual claims. The experiment measures
the closed/open/gap distribution on the rewritten WRN page against the original.

## Experiment results (2026-06-29, full WordNet+UMLS snapshot)

Rewrote the WRN first page into this style (`first-page-cnl.txt`, 63 short sentences) and ran the
coverage measurement (`wrn_first_page_over_full_lexicon`, `EIGENIUS_WRN_PAGE` override) against the
fresh `--umls-all` snapshot.

| Metric | Original page | v1 (parse-optimized) | v2 (faithful, R2) |
|---|---|---|---|
| units | 30 | 63 | 62 |
| **OOV (distinct)** | **13** | **1** (`hypermutable`) | **4** (`recq`, `double-stranded`, `pcr-based`, `hypermutable`) |
| parses (closed/ambiguous + open) | **0** | **9** (4 + 5) | **9** (3 + 6) |
| grammar-gap | 16 | 53 | 47 |
| missing-lexeme (units) | 14 | 1 | 6 |

**The faithfulness tax is low (v1 → v2).** Restoring every claim-bearing qualifier (R2) kept parse
count identical (9 → 9); the cost showed up almost entirely as **+3 OOV** (the restored specific terms
`recq`/`double-stranded`/`pcr-based` push their units into MISSING) — i.e. a *vocabulary* problem
(importable), not lost parses. So there is no real coverage-vs-faithfulness tension: **write faithfully
and pay the small vocabulary/grammar follow-on**, never trade meaning for parseability. v2 additionally
surfaces, as concrete follow-ons, the constructs a faithful version needs: **modal support**
(`can`/`may`/`would`), **comparatives** (`than`/`compared to`), and **comma-naming apposition**.

Two clear wins: **OOV collapsed 13 → 1** (controlled vocabulary works), and we got the **first real
parses (0 → 9)**. But 53/63 short, simple SVO sentences still GAP — and a targeted probe shows the
cause is **lexical, not grammatical**:

1. **`the` + plural noun gaps.** `the_subj`/`the_obj` are singular-only, so `the cancers affect WRN`
   → GAP, while `these groups are …` parses. English (and the CNL) uses "the X(plural)" constantly
   (`the MMR genes`, `the other DNA helicases`, `the lines from rare lineages`). **Fix: a plural
   `the` determiner** (small bootstrap add, like the numerals — reseed).
2. **Bare singular domain common-nouns used as names.** `MSI`, `MMR`, `Depletion`, `Toxicity` are
   count CNs in the lexicon, so bare (no determiner) they gap: `MSI arises` → GAP, but `WRN arises`
   → CLOSED (`WRN` is an HGNC **named individual**). **Verb frames themselves are fine** — `encodes`,
   `arises`, `exhibits`, `contributes to`, `occurs`, `responds` all parse with a name subject. **Fix:
   model domain abbreviations / mass concepts (MSI, MMR) as named individuals (or mass nouns)** — the
   gene-symbol-as-named-individual track extended beyond HGNC — OR write them with a determiner in the
   CNL. (`many`/`several`/`other`/`such` are NOT blockers — they parse as adjective-modified bare
   plurals → open.)

**Reframing (initial, then CORRECTED below):** the first read was "controlled English is
~grammar-complete; just two lexical fixes (plural `the` + named-individual abbreviations) stand
between us and the majority." A reseed-and-remeasure (below) showed that was **too optimistic**.

### Correction: Fix 1 had zero page impact; the residual is a diverse long tail (2026-06-29)

Reseeded with **Fix 1 (plural `the`)** baked in and re-measured the v2 page: **identical** to v2 (9
parses, 47 gap, 6 missing) — Fix 1 moved the page by **zero**, because every v2 unit using "the+plural"
is *also* blocked by something else (apposition, OOV `RecQ`, comparatives). Fix 1 is a correct fix
(verified on the small lexicon) but not a bottleneck on the faithful page. A per-unit probe then showed
the residual is a **diverse long tail**, ≥6 distinct causes (each confirmed by isolating sub-variants):
- **Bare domain CN** (MSI/MMR) — *Fix 2*, real: `MSI contributes to several cancers` GAP vs
  `WRN contributes …` open; `MSI occurs in cancers` GAP vs `WRN occurs …` CLOSED. ~8–12 units.
- **Verb-frame** — some verbs gap even with a NAME subject: `WRN results from …` GAP.
- **Compound as prep-object** — `… occur in cancers` CLOSED vs `… occur in nucleotide repeat regions`
  GAP (3-noun compound in a PP).
- **Numeral + of-PP** — `we identified three groups` open vs `… three groups of cell lines` GAP.
- **of-PP-modified determined NP as argument** — `an impairment of a DNA repair pathway affects WRN` GAP.
- **det + plural predicate-nominal** — `genes are microsatellites` CLOSED vs `these mutations are
  microsatellites` GAP.
- plus modals (R2-restored), comparatives, apposition, OOV (separate/known).

**Corrected conclusion:** the grammar *primitives* are mostly present, but their *interactions at
scale* (a 3-noun compound inside a PP inside an argument; beam pressure) plus **verb-frame coverage**
produce a steady drip of gaps. **No single fix clears the page.** Fix 2 (bare domain CN) is the largest
identifiable bucket and worth doing, but yields *partial* gains; the rest is incremental long-tail
work, not a two-fix finish.

## Faithfulness audit — the CNL rewrite vs the original (2026-06-29)

The rewrite gained parse coverage, but a meaning-level audit of the CNL against the original shows it
is **not** meaning-neutral. This is the **D61 faithfulness gap demonstrated in our own pipeline:
parse-faithful ≠ meaning-faithful.** Almost all changes are omissions of quantitative detail rather
than contradictions, but a few dropped qualifier words change the claim.

**Genuine factual distortions (meaning changed) — the load-bearing ones:**
- *"The other DNA helicases were not essential in MSI cell lines."* Original: *"none of the four other
  **RecQ** DNA helicases were **preferentially** essential."* Dropping **preferentially** turns a
  comparative claim (not *selectively* essential in MSI vs MSS) into an **absolute** one (not essential
  at all); dropping **four RecQ** generalizes from the RecQ family to **all** DNA helicases. The most
  significant discrepancy.
- *"The MMR genes are MSH2, MSH6, PMS2 and MLH1."* Original lists these as the MMR genes whose germline
  mutation causes Lynch syndrome — **not the complete set** of MMR genes. The rewrite implies these are
  the only MMR genes (false).
- *"Somatic MMR inactivation arises from hypermethylation of the MLH1 promoter."* Original: *"**typically**
  through" — one common mechanism; dropping **typically** states it as the **sole** cause.
- *"MSI arises from Lynch syndrome"* / *"Toxicity limits the use…"* — dropping the modal **can**
  overstates each claim (original: *"can arise"*, *"can be limited by"*).

**Losses of specificity (weaker precision, not contradiction):** `double-stranded DNA breaks` → `DNA
breaks` (severity lost); `PARP-1 inhibitors` → `PARP inhibitors`; `highly concordant with PCR-based MSI
phenotyping and with predicted MMR deficiency` → only `concordant with predicted MMR deficiency`.

**Pure omissions (dropped data, no contradiction):** essentially all numbers — cancer-type percentages
(colon 15%, gastric 22%, …), `45–60% do not respond`, screen sizes (517 / 398 lines), Q/P-values,
`51 MSI and 541 MSS`, `n=37 / n=91`, `14 MSI cell lines (six leukaemia, two prostate…)`, `median
0.56-fold fewer`, and `in vitro and in vivo`. Not wrong, but a reader couldn't reconstruct the
evidence base.

**Net:** a faithful plain-language simplification with **no fabricated facts**, but dropped qualifiers
— especially **preferentially** and the **RecQ** family restriction — yield claims **stronger or
broader** than the original supports.

### Lessons

1. **Parse-faithfulness ≠ meaning-faithfulness (the D61 gap, in vivo).** Every CNL sentence that
   parsed type-checks to a `Prop`, yet several encode a *distorted* claim. A kernel-passing certificate
   proves structural validity, not that the formalization captured intent — exactly the faithfulness
   gap D61 targets. An LLM rewrite-to-fit-the-parser **must** be paired with a faithfulness check
   (back-translation + consistency scoring against the source; D61 Phase 2), never trusted blind.
2. **Preserve qualifiers — they are load-bearing, and cheap to keep.** The distortions came from
   dropping words the parser *can* handle: modals (`can`/`may` → epistemic possibility, not assertion),
   scalar/comparative adverbs (`preferentially`, `selectively`, `typically`), scope restrictions
   (`four RecQ` → a family, not all), and severity (`double-stranded`). These are **style/rewrite
   discipline**, not parser limits. → New rule below.
3. **The quantitative omissions are partly *forced* by the parser** (it drops numbers — see
   `[[numbers_two_worlds]]` / `d52-d62-numbers-and-measurements.md`). So faithfully encoding *this*
   paper's evidence base needs the number/stat extraction path (D52 pieces), independent of grammar or
   CNL discipline.

### New style rule (added from the audit): keep the qualifiers

When simplifying, **carry every epistemic and scope qualifier** the parser supports, even though
dropping it would still parse:
- **Modals** `can`/`may`/`could` → keep the possibility (don't assert): write `MSI can arise from
  Lynch syndrome`, not `MSI arises from Lynch syndrome`. *(Modal support is itself a small grammar
  follow-on if not yet covered — track it; do not silently drop the modal to make a sentence parse.)*
- **Scalar/comparative adverbs** `preferentially`/`selectively`/`typically`/`highly` → keep them; they
  turn an absolute claim into the comparative/qualified one the source actually makes.
- **Scope restrictions** (`the four RecQ helicases`, not `the helicases`) → never broaden a set.
- **Severity / type specificity** (`double-stranded DNA breaks`) → keep the discriminating modifier.

A simplification may drop *data* (numbers, citations) — those are out of the claim by design — but it
**may not drop a qualifier that changes the claim's strength, scope, or modality.**
