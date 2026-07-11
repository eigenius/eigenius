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
5. **Adjectives & compounds.**
   - *Genuine* stacked attributive adjectives — each modifies the head **independently** (`a human
     colorectal tumour`); and noun–noun compounds (`cancer models`, `cell line`, `MSI cancer models`).
   - **A lexicalized compound modifier is ONE term, not a stack — HYPHENATE it.** Write
     **`synthetic-lethal targets`**, **`microsatellite-stable lines`** — *not* `synthetic lethal …`,
     `microsatellite stable …`. Hyphenation makes the parser read it as a single compound adjective (via
     the D63 hyphen morphology, like `double-stranded`), instead of a stack of independent adjectives it
     is not. Rationale: "Hyphenate lexicalized compound modifiers" below.
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
| **Novel / OOV or en-dash hyphenations** (`CRISPR–Cas9-mediated`; an en-dash `–`, not a hyphen) | An unknown head/base is OOV; the en-dash isn't the hyphen token. | Rephrase or drop the modifier. **But a hyphenated compound whose head is a known adjective now PARSES** (D63 morphology: `double-stranded`, `pcr-based`, `large-scale`, `synthetic-lethal`) — **prefer** hyphenation for lexicalized compound modifiers (DO §5), don't avoid it. |
| **Possessive ellipsis / heavy gapping**, fronted reduced clauses with complex complements | Limited; gapping beyond same-type `but not` isn't covered. | Use an explicit subject and a full verb in each clause. |
| **`and/or`** | Not a token; collapsing it to `and` overstates (requires *both*). | Write **`or`** — `logic:Or` is **inclusive** (true if either or both), which is exactly what `and/or` means. (Faithfulness rule, not just style — `and/or → and` is a meaning change; `and/or → or` is meaning-preserving.) |

## Hyphenate lexicalized compound modifiers (`synthetic-lethal`, not `synthetic lethal`)

A **lexicalized compound modifier** is a domain term of art whose parts do *not* combine compositionally
in general English — `synthetic lethal` is not "synthetic ∧ lethal" (a target that is artificial and
deadly); it is the attributive form of *synthetic lethality* (C4280020), the genetic concept where two
perturbations are each tolerated alone but lethal in combination. Left unhyphenated, such a term
**masquerades as a stack of independent adjectives**: `synthetic` and `lethal` each carry adjective *and*
noun senses, so the parser enumerates the Cartesian product of adjective/compound bracketings — a spurious
structural blow-up (D63 `d63-nominal-modification-normal-form.md` §1: S5 alone gave 12 skeletons), and the
"all-adjective" reading it settles on is the **wrong claim**.

**Rule.** Hyphenate a compound modifier when its parts would otherwise each be read as a separate
adjective (`synthetic-lethal`, `microsatellite-stable`). The D63 hyphen morphology reads it as one
compound adjective (head must be a known adjective — `lethal`, `stable` — exactly as `double-stranded`
works). This is *more* faithful (R2), not just faster: the claim is about one property, not a conjunction
of two. **Noun–noun compound modifiers** (`immune checkpoint blockade`, `DNA repair pathway`, `cell cycle
arrest`) are already handled by the compound rule and need not be hyphenated — the masquerade only arises
when a part has an adjective reading. A compound that is a lexicon **unit** already (noun `synthetic
lethality` → C4280020, `cell death`, `dna repair`) is fine as written; hyphenation is for the *modifier*
surface the lexicon doesn't carry.

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

### Full CNL v2 page re-measured (2026-06-30) — 9 → 33 parses

After the GH#97 lever work (countability lexicon + composed-mass shift, Lever-3 VP-adjunct prep-object
raise, mood-polymorphic VP-adjunct prep, adaptive beam-first widen, cross-POS prune, felicity-readback
made total) over a fresh `--umls-all` reseed, the whole 62-sentence v2 page measures (prune on):

| metric | start of session | now |
|---|---|---|
| parses (open + ambiguous) | 9 | **33** (30 open + 3 ambig) |
| grammar-gap | 47 | **23** |
| missing-lexeme (OOV) | 6 | 6 (`recq`, `double-stranded`, `pcr-based`, `hypermutable`) |
| encoded-closed / scale-bound | — | 0 / 0 |

Parse times mostly <1s; a few 14-token sentences ~35s (the noun-pile tax persists on the *longest*
sentences even with the prune). The 23 residual grammar-gaps cluster into **known backlog**, not new
mysteries:
- **Bare `MSI`/`MMR` abbreviation as argument** (~6) — the direct cost of removing the UMLS
  `MASS_FORMS` hack; routed to the **abbreviation/alias model** (#1), not yet built. (The hack parsed
  these as mass common nouns — the wrong shape.)
- **Comparatives** (~4: `compared to`, `greater … than`, `fewer … than`).
- **Multi-item comma lists** (~3: `colon, gastric, endometrial and ovarian cancers`).
- **Clausal-complement / apposition / long sentences** (the rest).

Measured WITH the cross-POS prune flag; without it coverage is lower and far slower. `encoded`=0 because
the carrier returns these as OPEN parses (referent/quant holes), not closed `Prop`s — resolution is the
downstream D64 step, not a parse failure.

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

## Final diagnosis (2026-06-30): the grammar grind is done; the residual is non-grammar

Worked the per-construction backlog (the "diverse long tail" above) systematically. The decisive
outcome is not "fixed N constructions" — it is that **three of the suspected grammar blockers turned
out to be non-issues**, and the genuine residual is **outside the grammar**. Each finding is witnessed
(small-lexicon kernel tests in `closed_class_determiners.rs`, full-lexicon probes).

- **Modals are not a gap — they already exist.** `can`/`could`/`may`/`might`/`must`/`will`/`would` map
  to `logic:Possible`/`Necessary`/`Will`/`Would`/`Should` as opaque sentential operators, cat
  `(S[dcl,fin]\NP)/(S[bse]\NP)`, with a passing test (`modal_can_wraps_the_proposition_in_possible`). I
  briefly built duplicate modal infrastructure before discovering this — reverted. *Lesson: retrieve
  before building.*
- **Composition-interactions are not a grammar gap.** Numeral+of-PP, of-PP-modified determined NP as
  argument, det+plural predicate-nominal, compound-as-prep-object — all **parse on the small lexicon**.
  Their full-lexicon GAPs are **beam/sense pressure** (the chart is beam-less over 7.6M entries), i.e.
  GH#97, *not* a missing rule. Witnessed by the beam-pressure probe (`novel therapies are needed for a
  gene`: GRAMMAR-GAP at page beam 64, `open×216` at cell_beam 1024).
- **Verb frames are not the blocker.** Every probed verb (`encodes`/`arises`/`exhibits`/`contributes
  to`/`occurs`/`responds`/`affects`/`results from`) parses **CLOSED with a name subject**. The earlier
  `WRN results from … GAP` was a sense/beam artifact, not a missing frame.
- **The one genuine isolated grammar-adjacent blocker: bare mass-noun (and adj+mass-noun) arguments.**
  Probe: `WRN affects mismatch repair` → open, but `WRN affects deficient repair` / `WRN affects
  deficient DNA mismatch repair` → GAP. The head `repair`/`instability` is grammatically a bare
  mass/uncountable noun used as an argument; the lexicon marks only 5 curated abbreviations `mass`.
  **Countability is lexical, not derivable from UMLS semantic types** — broad mass-marking by semantic
  type would wrongly break count uses (`a DNA repair pathway` is countable). So clearing this needs a
  **countability lexicon**, not a grammar rule. Fix 2 (the `mass` Num value + `bare_mass_nps` shift +
  the 5 curated abbreviations) is the *mechanism*; it gives +2 on the page and is correct, but the
  long tail behind it is a data-acquisition problem.
- **The CCG combinatory core is inert without re-shaped lexical families.** Implemented the full
  combinator set (>Bx/<B/<Bx crossed/backward composition + ENF guard) behind
  `EIGENIUS_COMBINATORY_CORE` and measured: **identical** parse counts core-on vs core-off on both the
  CNL and original pages. The combinators have nothing to compose because the lexical *families* are
  still type-indexed application categories, not the feature-shaped categories the combinators assume.
  Kept flag-off as a record (combinatory-core branch); a real port would re-shape the lexicon, a
  separate large effort. Confirms the earlier "combinators are formalism-bound, can't bulk-port"
  analysis.

**Strategic conclusion (witnessed).** The per-construction grammar grind has reached **diminishing
returns**: each remaining fix yields +0–2 units because the faithful page's units are **multiply
blocked**, and the grammar *primitives* are essentially complete. The three dominant residual blockers
are all **non-grammar**:
1. **Countability lexicon** — bare mass-noun arguments; needs countability data (not in UMLS/WordNet).
2. **Beam/sense at scale (GH#97)** — the beam-less chart over the full lexicon; the single biggest
   lever for long real sentences. *This is where coverage work should now go.*
3. **OOV domain vocab** — `recq`/`double-stranded`/`pcr-based`/`hypermutable`; a small mechanical import.

The remaining backlog items that *are* grammar (comparatives; `because`/`although`) are real but small
and, for `because`/`although`, **deliberately deferred** (they need the factive-dependent-signature
engine extension — proof-threading — not an opaque closed-class add; see
`d62-subordinator-design-findings`). They should not be added opaquely just to move the counter.

### Per-sentence diagnosis of the first 5 CNL v2 sentences (2026-06-30) — countability dominates

Ran a minimal-pair fragment ladder over the full lexicon (snapshot `wordnet-umls-2026-06-29`, cap-only)
for the first 5 CNL v2 sentences, varying one construction at a time against the known-good anchors
`genes are attractive targets` (CLOSED) / `genes affect cells` (open). Witnessed by the `#[ignore]`d
`diagnose_first_five_cnl` test in `crates/eigenius-wordnet/tests/db_backed_encoding.rs`.

**The 5 sentences reduce to exactly 3 root blockers** — and the dominant one is **countability**, not
the abstract "GH#97 beam/GH#93" prioritization:

| Sentence | Exact blocker | Category |
|---|---|---|
| S1 *Synthetic lethality is an interaction between two genetic events.* | `synthetic lethality` bare-singular common-noun **subject** | **bare-mass argument** |
| S2 *The co-occurrence of these two events leads to cell death.* | `leads to …` + `cell death` bare-singular object | **to-prep** (fixed) + **bare-mass arg** |
| S3 *Each event alone does not lead to cell death.* | `lead to …` + `cell death` bare-singular object | **to-prep** (fixed) + **bare-mass arg** |
| S4 *Scientists can exploit synthetic lethality for cancer therapeutics.* | `synthetic lethality` bare-singular common-noun **object** | **bare-mass argument** |
| S5 *DNA repair processes are attractive synthetic lethal targets.* | `DNA repair processes` 3-noun compound subject | **beam pressure (GH#97)** |

1. **Bare-mass argument (countability) — blocks 4 of 5 (S1–S4).** A bare *plural* common noun shifts to
   a deferred-quantification NP and parses (`genes affect cells` → open); a bare *singular* common noun
   in **argument** position does NOT (`genes affect lethality` GAP, `lethality affects cells` GAP,
   `a gene affects cell death` GAP). The same noun in **predicate** position is fine (`genes are cell
   death` CLOSED) — so it is specifically the NP-argument shift that's missing for bare singulars. The
   linguistically-correct fix is to treat bare singular **mass** nouns like bare plurals (the
   `bare_mass_nps` shift, which exists), gated on the noun being mass. **The blocker is coverage**: only
   5 UMLS abbreviations are mass-marked; `lethality`/`death`/`co-occurrence`/`instability` are
   unmarked. And countability is **not morphologically derivable** — `mutation`/`solution`/`function`
   are countable `-tion`/`-ion` nouns, `inactivation`/`recombination`/`co-occurrence` are mass; same
   suffix, opposite countability. So this needs a **countability lexicon** (curated for the domain, or
   an external source like Wiktionary `uncountable` / COMLEX `NUNCOUNT`), not a heuristic.
2. **to-preposition — blocks S2, S3.** `lead(s) to …` needs `to_prep`; **already implemented** in the
   working tree (closed-class `to_prep` + `prep_to` axiom + kernel test `to_preposition_parses`), not
   yet in the snapshot — pending reseed. (Even with `to`-prep, S2/S3 still need the `cell death`
   bare-mass object — blocker 1.)
3. **Beam pressure (GH#97 Lever B) — blocks S5 only.** `DNA repair processes are attractive targets` is
   GRAMMAR-GAP at the page beam (64) but **open×196 at cell_beam=1024** — the 3-noun compound subject's
   sub-constituents are beamed out at 64 (the 2-noun `repair processes …` already hits `open×64`, the
   beam ceiling). A scale issue, not a missing compound rule.

**Revised priority (witnessed, supersedes the abstract ordering above):** the single highest-leverage
lever for the CNL is the **countability lexicon** (bare-mass arguments, 4/5 sentences) — *not* GH#93 or
Lever B. `to`-prep is done (reseed to realize it). GH#97 Lever B / beam is real but, for these
sentences, blocks only S5.

### Countability lexicon BUILT + reseeded — 4/5 now parse; blocker shifted to beam (2026-06-30)

Implemented the external countability lexicon (the chosen path) and re-measured against a fresh
`--umls-all` reseed (`wordnet-umls-2026-06-30`) carrying it:

- **Data:** Wiktionary `Category:English uncountable nouns` (CC-BY-SA) ∩ WordNet noun lemmas =
  **32,120 mass lemmas** (`scripts/provision-countability.sh` → `references/wiktionary/uncountable-nouns.txt`).
  Precision validated: `lethality`/`instability`/`death`/`co-occurrence`/`apoptosis`/`toxicity` tagged;
  `gene`/`function`/`pathway`/`cell` not. `mutation` is tagged (it has a mass sense) — harmless under
  the **additive** design.
- **Importer:** `wordnet-import --countability <list>` emits an *additive* `cat_n(C, mass)` entry
  ALONGSIDE the count entry for each flagged lemma (so `a mutation`/`three mutations` keep parsing while
  bare `mutation` shifts). The `--all` reseed emitted **43,309** additive mass entries; the WordNet
  layer validated clean against the `mass`-`Num` bootstrap.
- **Parser fix (necessary complement):** the composed-cell bare-NP shift in `lookup.rs` ran
  `bare_plural_nps` but not `bare_mass_nps` — so a bare *leaf* mass noun (`lethality`) shifted but an
  *adjective-modified / compound* one (`synthetic lethality`, `cell death`) did not. Added the mass arm
  (symmetric with the leaf path). Witnessed: `genes affect synthetic lethality` GAP → **open×12**.

**Result (fresh snapshot).** Bare-mass arguments now parse (`genes affect lethality` / `lethality
affects cells` / `a gene affects cell death` — all open). For the 5 sentences, at the page beam (64)
**S2 parses (open×20)**; at a **wide beam (1024) 4 of 5 parse** — S1 (open×183), S2 (open×200),
S3 (open×192), S5 (open×210). The countability + `to`-prep + composed-mass-shift fixes **resolved the
grammar blockers** for S1/S2/S3/S5; those four gap at the page beam **only from beam pressure (GH#97)**.

**The blocker has shifted: countability → GH#97 beam.** With the grammar gaps closed, the binding
constraint for S1/S3/S5 is now the page-beam cell cap (they parse only when the beam is widened). GH#97
Lever B (exact mid-chart felicity pruning, gated on GH#93) and/or a larger page beam is now the lever.

**Reconciliation — removed the earlier UMLS `MASS_FORMS` stopgap (2026-06-30).** The pre-session
"Fix 2" hardcoded 5 acronyms (`MSI`/`MMR`/`MSS`/`DNA`/`RNA`) as mass in the UMLS importer. With the
general lexicon built, that hack is removed: `DNA`/`RNA` are WordNet noun lemmas already mass-marked by
the `--countability` path (so nothing is lost), and `MSI`/`MMR`/`MSS` are **not mass nouns** — they are
abbreviations for phenomena (`microsatellite instability`, whose head `instability` IS mass via the
lexicon). Their bare-argument use belongs to the **abbreviation/alias model** (#1), not a mass
common-noun shim — so they now route there rather than being propped up by a curated list. Net: one
general, externally-sourced countability mechanism (WordNet side); no per-importer hardcode.

**One residual real gap: S4.** `Scientists can exploit synthetic lethality for cancer therapeutics.`
gaps **even at wide beam**, localized to a **compound noun as a *preposition* object**: `for therapies`
(single bare plural) works, `cancer therapeutics` as a *direct* object works (open×36), but `for cancer
therapeutics` gaps. Single-noun prep-objects and compound direct-objects both parse — only
compound-as-prep-object fails. Likely a prep-object/composed-shift interaction (or a beam effect even at
1024); needs small-lexicon isolation to call definitively. Tracked as a follow-up.
