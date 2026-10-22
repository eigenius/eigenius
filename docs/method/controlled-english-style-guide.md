# Eigenius Controlled English — a style guide for parser-faithful scientific prose

*A controlled natural language (CNL) for writing factual scientific claims that the Eigenius DCG/CCG
parser fully covers, so the encoding captures the **claim** (a kernel-checked `Prop`), not an
approximation. Grounded in the parser's actual capabilities, not aspiration.*

*Originated as a D62 experiment (`2026-06-29`) that rewrote the WRN first page into this style and
measured the coverage change. Moved here `2026-08-19` and trimmed to the guide: the experiment log it
carried is superseded — the corpus now parses 62/62 with `grammar-gap 0` and `missing-lexeme 0`
(`experiments/parsing/baseline.json`), so the June coverage figures describe a parser that no longer
exists. **Corrections applied in the same pass are marked ⚠ below.** This is an authoring guide, so it
is expected to drift as the grammar grows; check a claim against the baseline before relying on it.*

*Revised `2026-09-20` for measured quantities, and `2026-09-27` when D93 (units of measure) and D95
slices 1–5 (quantities in the parser) made them parse. The claims about them were measured over the
full lexicon (`wordnet-umls-aligned-2026-09-27`, cap-only). **What is still marked 🔜 is NOT live**: a
temperature or any other difference needs D95 slice 7. This guide does not state aspiration as
capability.*

*Revised `2026-09-29` for D95 slice 6 — bounds, approximations, ranges, bound symbols and scientific
notation parse — and for the owner's decision that **a plain number is exact**: a bound is written
out (Measured quantities, rules 7–9). The slice-6 claims are checked by `kernel/tests/` over the
bootstrap chain; the full-lexicon run waits for the reseed after D95 slice 8. Revised again the same
day for D95 slice 7: counts, proportions, number words, a determiner before a numeral, and counted
conjuncts sharing a head (DO item 3; Measured quantities, rule 10); and for slice 8: offsets
(`72 h after transduction`), a deadline (`by three weeks`), `every N unit`, the pseudo-partitive, a
fronted adjunct, `per` after a count, and a list with its unit once (DO items 11 and 13; Measured
quantities, rules 11 and 12); and for slice 9: a factor on a count comparative, `a median` before it,
and `compared to` for `than` (Measured quantities, rule 13).*

**Revised `2026-10-13`** against the UAB survey §4 step 5 measurements — the first revision driven
by a corpus other than WRN, and the first to CORRECT claims rather than only add them. Every row
below is now backed by a named probe (see "Each row is a probe", at the end), because the previous
posture — "expected to drift as the grammar grows; check a claim against the baseline" — put the
burden on the reader and left the guide six months stale. Three DO items claimed coverage the
parser does not have, and one DON'T row instructed authors to DESTROY content the parser now
carries:

- **`because` / `although` flipped from DON'T to DO.** The row said "subordinators unbuilt → split +
  use a transitional". D62 2d Fork A built them (`logic:Because : Π(p q:Prop) → p → q → Prop`), so
  splitting now DISCARDS the causal relation instead of working around a gap. Keeping the
  subordinator is preferred.
- **Coordination, relative clauses and clausal complements were over-claimed** — NP coordination
  fails in SUBJECT position, and the ZERO relative and ZERO complementizer both fail. See the
  corrected DO items 4, 7 and 10.
- A dozen measured DON'Ts were missing, the largest being that the lexer DELETES a clause-position
  `:` or `;`.

**Revised `2026-10-22`: a measured value is a claim, and now parses.** D95 slice 7 decision 6's
closing gloss — a bare dimensionless number is a statistic kept out of the claim — was narrowed to
TEST statistics, and `lexicon:cat_numpred` gives the copula a route to `has_quantity(x, u"1", q)`.
R3 and the "Test statistics" DON'T row both told an author to drop exactly what the certification
corpus is mostly made of: 17% of every `observed` claim. On the 1,056-sentence CNL corpus, parsed
485 → 495 at +0.9% readings, with D95's own `at least 1,000 cells` control identical to baseline.

**Revised `2026-10-15`, the final revision before the rewrite it governs.** Three changes, all from
measurements taken `2026-10-14`. (1) **R1 now carries a token budget** — under 31 — because length is
the only variable measured on the XIAP page that separates parsed units from failing ones, and R1 now
explicitly outranks every DON'T row. (2) **A new R3: a citation is provenance, not a claim**, after 64
of the certification log's 427 gold spans turned out to be bibliographic records. (3) **A new "How to
apply this guide"** carrying the census that demotes the DON'T tables to a correctness filter: every
countable row sits at the 86% base gap rate and 46% of failing units carry none of them, so a
row-by-row pass moves nothing. The tables are unchanged in content; what changed is their standing.

**Revised `2026-10-14`: the ACTIVE participial post-modifier moved from DON'T to DO.** The `ger` arm
of `reduced_relative` built it, so `a patient carrying the variant` composes and its DON'T row is
gone. This is the second row in two revisions to flip because the grammar grew, and the pattern is
worth naming: **a DON'T row is a rewrite the author must perform at every occurrence**, so its real
cost is the occurrence count, not the row count. This one stood over ~15 of the XIAP page's 325
units while converting 2 of them outright — and its own remedy warned that the obvious rewrite
(splitting) changes the claim. A grammar fix that deletes such a row is worth more than its
converted-unit count suggests.

## Purpose & posture

The parser is the oracle: a sentence either composes into a kernel-checked typed tree or it does not.
Rather than bend the grammar to arbitrary journal prose (long, compound, statistic-laden), **write the
science in the subset the parser covers**. This matches the encoding objective — we want the
*load-bearing factual claims* as checkable `Prop`s; rhetorical packaging, inline statistics, and
citations are out of the claim by design (D62 S0 routes them out).

Three rules sit above everything else, and **R1 outranks every DON'T row in this guide** — see "How
to apply this guide" below for the measurement that puts it there.

- **(R1) One claim per sentence, and keep it under 31 tokens.** Almost every grammar gap below is
  dissolved by splitting a compound journal sentence into several short factual ones. The budget is
  measured, not a style preference: over the 325 prose units of the UAB XIAP report
  (`2026-10-14`), units that produced a reading ran a **median of 11 tokens and NONE exceeded 31**,
  while units that failed ran a median of 22 and up to 59. Length is the only variable measured on
  that corpus that separates the two. It is necessary and not sufficient — 69% of the failing units
  were already under 31 tokens — so the budget is a floor to clear before the rest of the guide
  applies, not a guarantee.
- **(R2) Faithfulness over parseability — never drop a *qualifier* to make a sentence parse.** A
  simplification may drop *data* (numbers, citations, figure refs — out of the claim by design), but it
  **may not** drop a word that changes the claim's **strength, scope, or modality**: modals
  (`can`/`may`), scalar/comparative adverbs (`preferentially`/`selectively`/`typically`/`highly`),
  scope restrictions (`the four RecQ helicases`, not `the helicases`), or severity/type specificity
  (`double-stranded`). If keeping a qualifier means the sentence does not yet parse, **keep it anyway
  and record the gap** — a faithful un-parsed claim is a tracked to-do; a parsed distorted claim is a
  silent error (the D61 faithfulness gap). See the audit + rule at the end of this note for why.
- **(R3) A citation is provenance, not a claim — do not write one as a sentence.** A reference-list
  entry (author list, title, journal, year, pages, PMID) has no truth condition, so there is nothing
  for the parser to check and nothing for the kernel to hold. It belongs in the claim's provenance
  fields, which is where a certificate already keeps it. Measured on the XIAP certification log
  (`2026-10-14`): **64 of its 427 gold claim spans are bibliographic records**, 61 of them registered
  under the `observed` kind, and 56 carry a `PMID`. The harness classifies all 64 `non-prose`, which
  is correct — they are 15% of the register and unparseable by construction. The same applies to a
  span that covers SEVERAL claims (`compressed`) or states why a claim is not checkable (`exempt`):
  0 of those 25 spans parse, because neither is a single assertion. Represent them as structure.
  - ⚠ **A MEASURED VALUE is not a citation and not a statistic — keep it in the claim.** `The
    observed-to-expected ratio for that window is 0.94.` parses (`2026-10-22`), to
    `has_quantity(the(ratio), u"1", 0.94)`. The line to draw is whether the number describes the
    SUBJECT MATTER or the inference procedure: an observed-to-expected ratio, a predictor score and
    an allele frequency are claims with truth conditions; `P`, `n` and `Q` qualify a claim and are
    not one. Measured: **53 of the 427 gold claim spans carry a bare dimensionless decimal, 38 of
    them `observed` — 17% of every `observed` claim**, each with a `reproduce:` line. Routing those
    to provenance would leave the kernel checking a judgement in place of the measurement that was
    reproduced.

## How to apply this guide

**Rewrite for LENGTH first. Use the DON'T rows as a correctness filter while you do, not as the thing
that decides what to rewrite.** This ordering is measured, and the measurement is the reason the
guide now says it (`2026-10-14`, the 325-unit XIAP page against its 279 grammar gaps):

| | gap units | all units | gap-rate |
|---|---:|---:|---:|
| clause-position `:` or `;` | 78 | 93 | 84% |
| em-dash appositive | 66 | 76 | 87% |
| parenthetical aside `(…)` | 61 | 73 | 84% |
| en-dash inside a word | 21 | 25 | 84% |
| **base gap rate** | **279** | **325** | **86%** |

Every mechanically-countable DON'T row sits AT the base rate, so carrying one makes a unit no more
likely to fail than a unit without it. And **46% of the failing units carry none of the four**, at the
same median length as the rest — they are not the short, simple residue. The rows are frequent, not
blocking; a row-by-row pass over the document would feel productive and move nothing.

What a row IS good for is the edit you make once you have split a sentence, because a row names a
construction with a known-wrong or known-absent analysis. Two consequences for how to read the
tables:

- **A row's cost is its OCCURRENCE count, not its row count** — it is a rewrite performed at every
  instance. When the grammar absorbs a row, that whole burden disappears: the active participial row
  (withdrawn `2026-10-14`) stood over ~15 of the page's 325 units.
- **Prefer a row whose rewrite is mechanical.** Where a row's own recipe warns that the obvious
  rewrite changes the claim, R2 governs and the safe move is often to leave it and record the gap.

## DO — constructions the parser covers

1. **Subject–verb–object, one clause.** `WRN is essential in MSI models.` `Depletion of WRN promotes
   apoptosis.` Present tense (`affects`/`affect`) or simple past (`affected`, `was`/`were`).
2. **Predicate nominals & adjectives.** `WRN is a vulnerability.` `WRN is a drug target.` `The
   dependency is selective.` Copula present/past: `is`/`are`/`was`/`were`.
3. **Determiners.** `a`/`an`/`the`/`every`/`each`/`all`/`some`/`no`, and numbers — `one` … `ten` in
   words, any number in digits (`three genes`, `3 genes`, `1,000 cells`), which read alike.
   - **A determiner before a number**: `the four other RecQ helicases` states the four; `these two
     genetic events` points back at exactly two earlier referents. Today the only set a demonstrative
     resolves to is a run of findings of one kind (D68), so `these four lineages` after four named
     kinds stays open — write the NP out (`the four lineages`) where no such run precedes it.
   - **Counted conjuncts may share their head**: `five MSS and five MSI cell lines` is five of each,
     and `15% of colon, 22% of gastric and 12% of ovarian cancers` a proportion of each. Each conjunct
     is a number or a proportion with the modifiers of its own kind; the head is written once.
   - **A plain count is exact** — `5 MSI cell lines` states five (decided `2026-09-29`; every plain
     count in the WRN paper is exact), as `has_count(…, 5)`. Write a bound out: `at least 1,000
     cells`, `more than one MMR gene`, `5 or more cell lines`, `fewer than 5 cells`, and a count range
     with an en-dash, `4–7 foci` (rule 7 under Measured quantities).
   - A bare number is a count, not a measurement: `incubated at 37` has no parse — write the unit.
   - ⚠ **Bare plurals and bare mass nouns now CLOSE, not open.** `Cancers exhibit defects.` composes as
     a kind predication (`kind_of`), not as a deferred quantifier. Prefer the bare form for
     kind-level claims — it is the shorter and the closed one.
   - ⚠ **Demonstratives are anaphoric, not determiners.** `this`/`that`/`these`/`those` open a
     RESTRICTOR-TYPED referent hole (`lexicon:anaphor_of`), so `These findings show …` resolves only
     against an antecedent that is itself a finding, earlier in the same document. A demonstrative in
     the first sentence, or one whose restrictor matches nothing prior, leaves the sentence OPEN.
     Write the full NP when there is no antecedent to point at.
4. **Coordination — in OBJECT position, and between like non-NP categories.** `and`/`or`; comma
   lists `X, Y and Z`; sentence-level `S but S`.
   - ⚠ **NP coordination fails as a SUBJECT.** `The study measured the result and the control.`
     parses; `The result and the control are clear.` does NOT. Write two sentences, or move the
     coordination into object position.
   - **Contrastive `X but not Y` and `X rather than Y`** are the same operator (`X ∧ ¬Y`) and work
     between **adjectives, verbs or PPs** — `clear but not ambiguous`, `measured rather than
     assumed`, `in the liver rather than in the kidney`. ⚠ **Between two NPs neither parses**:
     `measured the result rather than the control` has no parse. Split it.
   - ⚠ **`both … and` over two VPs has no parse** (`both bounds the argument and reinforces the
     conclusion`). Write two sentences.
   - ⚠ **VP coordination under one auxiliary has no parse** (`has been settled and is recorded
     below`). Repeat the auxiliary, or split.
   - ⚠ **Unlike categories cannot coordinate** (`It is the same residue, carried by four men, and
     benign.`). One category per coordination.
5. **Adjectives & compounds.**
   - *Genuine* stacked attributive adjectives — each modifies the head **independently** (`a human
     colorectal tumour`); and noun–noun compounds (`cancer models`, `cell line`, `MSI cancer models`).
   - **A lexicalized compound modifier is ONE term, not a stack — HYPHENATE it** when the lexicon does
     not already carry it. Write **`microsatellite-stable lines`**, not `microsatellite stable …`.
     Hyphenation makes the parser read it as a single compound adjective (via the D63 hyphen
     morphology, like `double-stranded`) instead of a stack of independent adjectives it is not.
     ⚠ `synthetic lethal` is no longer an example: it is now a curated multiword entry for C4280020
     (`experiments/lexicon-align/atom-overrides.json`), so both spellings work. That is the better fix
     when a term recurs — hyphenation is the authoring workaround for terms the lexicon lacks.
     Rationale: "Hyphenate lexicalized compound modifiers" below.
6. **Prepositional phrases.** `of`/`in`/`for`/`with`/`on`/`from`/`within`/`between`, as noun
   post-modifiers (`a biomarker of dependency`) and verb adjuncts (`essential in MSI models`). The
   object may be a determined NP (`within a gene`, `for tumours`).
7. **Relative clauses — ALWAYS with the relativizer written.** Restrictive `the gene that affects X`
   / `which affects X`; non-restrictive `WRN, which encodes a helicase, is essential.`
   - ⚠ **The ZERO relative has no parse.** `The resource I could query is reliable.` fails where
     `The resource that I could query is reliable.` parses (CLOSED×27). Never drop `that`.
   - **The agentless reduced relative now parses** (D62 2d short-passive lift): `the resource
     queried in the study`, `the method used in the study`, `the release queried recently`, and bare
     `the resource queried`. So a past-participial post-modifier needs no rewrite.
   - **The ACTIVE participial post-modifier now parses too** (`2026-10-14`, the `ger` arm of
     `reduced_relative`): `a patient carrying the variant` (CLOSED×2), `a method resolving the
     question` (CLOSED×4), `every method placing the target at the position` (CLOSED×16). It derives
     the SAME restrictor as the relativizer form — `Σx:method. resolve(the question, x)` — so the two
     are interchangeable and neither needs rewriting to the other.
   - ⚠ **Still write the relativizer when the participle is INTRANSITIVE and the verb has an
     adjective of the same spelling.** `a patient waiting` resolves to the adjective `waiting`, not
     to the participle, and `the gene encoding a helicase` to a compound noun pile. The `ger` entry
     does not reach the chart for those surfaces. Where the verb has no adjective or noun homograph
     (`carrying`, `resolving`, `arriving`, `placing`), the bare participle is fine.
8. **Passive.** `WRN was depleted.` `Apoptosis was promoted by depletion.`
9. **Negation.** `WRN does not affect MSS models.` `The activity is not essential.`
10. **Clausal complements (report verbs) — ALWAYS with `that`.** `These findings show that WRN is a
    vulnerability.` ⚠ **The ZERO complementizer has no parse**: `The study mentioned the result was
    clear.` fails where the `that` form parses (CLOSED×8).
11. **Transitional adverbs** (sentence-initial): `Thus,` `Therefore,` `Hence,` `Moreover,`
    `Similarly,` `Notably,` — transparent (they don't change the claim). **One adjunct may open the
    sentence**, with or without a comma, and reads as it does after the verb: `After 24 h, the medium
    was replaced.` = `The medium was replaced after 24 h.` A second opening adjunct (`Then, 7 days
    after transduction, cells were collected.`) has no parse: keep one, move the other after the
    verb.
12. **Light verbs** that exist in the lexicon, e.g. `gives rise to`.
13. **Measured quantities** after a preposition, before a noun, or after the copula: `kept at
    37 °C for 1 h`, `10 μM etoposide`, `The incubation was 1 h.` — and bounded or ranged in the same
    places: `at less than 37 °C`, `at 37 °C or higher`, `>90% infection efficiency`, `for 2–3 h`; before
    `after`, `before`, `post` and `later` (`purified 72 h after transduction`, `fixed 2 days later`),
    after `by` and `every` (`recovered by three weeks`, `changed every 3 days`), and before `of` and a
    noun (`300 μl of CellTiter-Glo`). A factor goes before a count comparative (`0.56-fold fewer
    deletion mutations`). See "Measured quantities" below.

14. **Factive subordinators — `because`, `although`, `while`, `though`, `whilst`** (D62 2d Fork A,
    `2026-10-13`). Write them; do NOT split them. Each presupposes both clauses, and the parser
    builds `logic:Because(p, q, h_p, h_q)` where `h_p`/`h_q` are HYPOTHETICAL proofs exported as
    proof obligations for the reasoning layer. Splitting into two sentences would discard the
    relation, which is the content.
    - **A factive clause yields an OPEN parse, never a closed one** — the obligations cannot be
      discharged by the parser (felicity is not truth). That is correct, and it counts as a parse
      under the success criterion below.
    - ⚠ **`unless` and `whereas` also parse** — `unless` is `¬S₂ → S₁` (native implication),
      `whereas` denotes what `but` denotes.
    - ⚠ **The TEMPORAL subordinators do NOT**: `since`, `until`, `when`, `once` have no account at
      all — EigenTT has no time indexing. Restate as separate claims with an explicit order.
15. **Possessive determiners, either head number.** `my`/`your`/`our`/`his`/`her`/`its`/`their`
    before a singular or plural head (`my gene`, `my genes`). The possessor is a referent hole, so
    the sentence is OPEN until an antecedent resolves it — as for a pronoun.
16. **Indefinite pronouns, BARE or with a verb.** `Nothing resolves the question.` `Everything is
    clear.` `Someone raised it.` They are quantified NPs, not entities — the negative forms build
    `∀x. ¬V(x)`.
    - ⚠ **With a post-modifier they have no parse**: `Everything in Findings 1–7 is inference.`
      fails. Write the restriction as its own claim, or name the set (`Findings 1–7 are inference.`).
    - ⚠ **`neither` as a bare subject has no parse** (`Neither is evidence that …`) — it presupposes
      a two-member set the grammar cannot restrict to. Name both members.
    - ⚠ **`both of those` has no parse** either; `none of the methods` does.
17. **`cannot`** now reads as `can` + `not` (lexer, `2026-10-13`), so it needs no rewrite.

## DON'T — and how to rewrite it

| Avoid (journal style) | Why | Rewrite recipe |
|---|---|---|
| **Test statistics** (`n = 37`, `P = 4.2 × 10⁻¹³`, `Q = 4.8 × 10⁻²⁴`) | Out of the claim **by design** — a statistic qualifies a claim, it is not one. Routed to a D52 record. **Narrowed `2026-10-22`:** this covers a property of the INFERENCE PROCEDURE only. A measured quantity of the subject matter — an observed-to-expected ratio, a predictor score, an allele frequency — is a claim and now parses (`The ratio is 0.94.` → `has_quantity(…, u"1", 0.94)`); keep it. | State the **qualitative** claim for a test statistic; the statistic lives elsewhere. `… showed greater dependence …`, not `(n = 37; P = …)`. Do NOT apply this to a measured value. |
| **A quantity as a verb's object** (`contained 4 μg`, `reached ~100 mm³`) | A quantity is not a noun phrase. It composes where a word takes one (DO §13); an imported verb takes a noun phrase (D95 slice 8, decision 7). | Name the quantity: `reached a volume of about 100 mm³`, `contained 4 μg of puromycin`. |
| **A preposition with a clause as its object** (`96 h after adding doxycycline`) | A preposition takes a noun phrase; a gerund clause has no parse, with or without the quantity. | Nominalize: `96 h after the addition of doxycycline`. |
| **`the` before a mass noun** (`The viability was assayed.`) | `the` has entries for a singular and a plural count noun only. | Write the bare mass noun: `Cell viability was assayed.` |
| **A range with the unit twice, or in words** (`37 °C–39 °C`, `between 37 °C and 39 °C`) | The range grammar reads a digit pair with one unit or `%` after it (D95 slice 6). An en-dash pair with no unit is a count range (`4–7 foci`, slice 7); a hyphen pair with no unit is not a range, since it may be a catalogue number (`926-68021`). | Write the pair with the unit once, after it: `37–39 °C`, `20–30%`. Do **not** collapse a range to one endpoint or to a midpoint — that changes the claim, which R2 forbids. |
| **A plain number meant as a bound** (`5 cell lines` for "five or more", `37 °C` for "37 °C or above") | A plain number is exact (Measured quantities, rule 7). | Write the bound: `at least 5 cell lines`, `37 °C or higher`. |
| **Parenthetical asides / inline abbreviations** (`(MSI)`, `(PARP-1)`, `(Fig. 1a)`) | Asides are dropped; the parenthetical can't be a claim. | Introduce an abbreviation in its **own** sentence, or just use one form consistently. Drop figure/citation refs. |
| **Telegraphic caption annotations** (`Scale bar, 50 μm`; `pH 7.5`; `(1,200 V, 20 ms, 2 pulses)`) | Not sentences — a label and its value, with no verb. Figure legends and instrument settings are written in an elliptical register the sentence grammar does not cover. | Expand to the sentence it abbreviates: `The scale bar is 50 μm.` A parameter list becomes one sentence per parameter. This is **register**, not content — nothing is added or dropped, so R2 is satisfied. |
| **Em-dash appositives** (`—an interaction…—`) | Not covered; the dash content is dropped. | Split into separate sentences: `Synthetic lethality is an interaction between two genetic events. …` |
| **Long multi-clause sentences** (relative + subordinate + parenthetical stacked) | Each clause must compose; one gap kills the unit. **This is the dominant variable** — parsed units measured a median 11 tokens with none over 31, failing units a median 22 and up to 59 (XIAP page, `2026-10-14`). | Split per R1, to one claim and under 31 tokens each. Splitting is the lever; the other rows in this table are the filter applied afterwards. |
| ~~**`because` / `although` subordinate clauses**~~ — **WITHDRAWN `2026-10-13`, now a DO (§14)** | They were OOV; D62 2d Fork A built them as factive dependent relations. | **Do not split them.** Splitting discards the causal or concessive relation, which is part of the claim. The old recipe ("split + use a transitional") actively destroys content the parser now carries. |
| **Cross-type `but not`** (`required the helicase activity … but not its exonuclease activity` — different kinds) | The two objects must be the same category. | Split: `MSI models required the helicase activity of WRN. MSI models did not require the exonuclease activity of WRN.` |
| **Deeply-embedded / determined-subject pied-piping** (`the way in which the co-occurrence leads…`) | Only simple/name-subject pied-piping is covered. | Rephrase as a separate clause: `The co-occurrence leads to cell death.` |
| **Novel / OOV or en-dash hyphenations** (`CRISPR–Cas9-mediated`; an en-dash `–`, not a hyphen) | An unknown head/base is OOV; the en-dash isn't the hyphen token. | Rephrase or drop the modifier. **But a hyphenated compound whose head is a known adjective now PARSES** (D63 morphology: `double-stranded`, `pcr-based`, `large-scale`, `synthetic-lethal`) — **prefer** hyphenation for lexicalized compound modifiers (DO §5), don't avoid it. |
| **Possessive ellipsis / heavy gapping**, fronted reduced clauses with complex complements | Limited; gapping beyond same-type `but not` isn't covered. | Use an explicit subject and a full verb in each clause. |
| **`and/or`** | Not a token; collapsing it to `and` overstates (requires *both*). | Write **`or`** — `logic:Or` is **inclusive** (true if either or both), which is exactly what `and/or` means. (Faithfulness rule, not just style — `and/or → and` is a meaning change; `and/or → or` is meaning-preserving.) |

### Added `2026-10-13` — measured on the UAB survey corpus

| Avoid | Why | Rewrite recipe |
|---|---|---|
| **A clause-position `:` or `;`** (`The residue is buried: it sits in the core.`) | The lexer emits `,` as a token but **DELETES** `:` and `;`, handing the grammar two independent clauses spliced with no connective. Measured: 68 gap units carry one. | Write two sentences. (Measured: splitting them mechanically recovers 11 pieces of 145 — real, and far from sufficient.) |
| **A label colon** (`Searched: holds no score set.`; `The original description: establishes loss of function.`) | NOT two clauses — a colon joining an annotation LABEL to its predicate. Splitting it gives a bare NP and a bare VP, neither of which parses; leaving it gives the splice above. | Expand to a sentence: `The search holds no score set.` This is the register fix the "telegraphic caption" row prescribes, applied to annotation lists. |
| **A comma appositive** (`CADD v1.6, the release queried here, gives X`) | No rule covers it. `appose_group` handles the `<common noun> <Name>` designation (`gene MSH2`), a different construction. | Two sentences: `CADD v1.6 is the release queried here. CADD v1.6 gives X.` This one splits faithfully. |
| **A NOUN comparative** (`greater dependence than the control`) | No parse. ADJECTIVE comparatives do parse — `clearer than`, `more conservative than` — and so does a plain object predicative (`rate the variant damaging`), but not `rate the variant worse than the control`. | Recast onto an adjective: `The dependence is greater than the control's.` |
| **A free relative** (`Absence is what a neutral variant looks like.`) | No parse — a clause used as an NP, with a stranded preposition. | Name the thing: `Absence is the normal pattern for a neutral variant.` |
| **`without` + gerund** (`searched without finding the variant`) | A preposition takes a noun phrase; the gerund clause has no parse (the same reason as the existing "preposition with a clause as its object" row). | Two clauses: `The study searched. The study did not find the variant.` |
| **A parenthetical adverb between commas** (`It is therefore, empirically, compatible.`) | No parse. A sentence-initial transitional adverb IS covered (DO §11). | Move it to the front: `Empirically, it is therefore compatible.` |
| **Sentence-initial `So`** (`So this is the complete set.`) | No parse — though sentence-initial `Yet` parses (CLOSED×36), as do the DO §11 transitionals. | Use a covered transitional: `Therefore, this is the complete set.` |
| **A fronted conditional** (`If the assay fails, the result is void.`) | No parse in the fronted position. | Put the subordinate clause second: `The result is void if the assay fails.` |
| **An imperative** (`Report the result.` / `Do not report the result.`) | A DECLARED deferral, not an oversight: `lexicon:Mood` has `imp`, and ⟦S[imp,_]⟧ fails closed pending D63 Slice 5. core-en has no imperative either. | Restate as a declarative about what is to be done: `The result should be reported.` |
| **An embedded question** (`reports where the gene is expressed`, `whether`, `how`, `why`) | Gated on the same Slice 5 interrogative denotation as the imperative. | Nominalize: `reports the location of expression`. |
| **A verbless unit** (`The source of the results.`; `ESM1b.`) | A unit must be a sentence; `Parser::parse` returns full-span `S` only. Measured: framing such units as NP subjects recovers them at no better than the length baseline, so this is a register fix, not a grammar gap. | Write the sentence it abbreviates, or route it out of prose (a bibliography annotation is not a claim). |

## Measured quantities (D93 / D95)

A measured quantity is `⟨numeral⟩ ⟨unit symbol⟩`. It is read as one token and converted to base units
(D93): `37 °C` is 310.15 K, `1 h` is 3600 s. It is **not a noun phrase**, so it composes only where
something takes it:

- **after a preposition**: `at`, `for`, `in`, `with` and `after` as verb adjuncts (`kept at
  37 °C for 1 h`, `resuspended in 50 μl`, `harvested after 72 h`), and `of`, `with` and `at` after a
  noun (`a dose of 5 mg/kg`, `a volume of 2,000 mm³`);
- **before a noun**: `10 μM etoposide`, `a 24 h incubation`, `10% FBS`;
- **after the copula**: `The incubation was 1 h.`, `The temperature was 37 °C.`;
- **before a temporal preposition**, as its offset: `purified 72 h after transduction`, `6 h before
  collection`, `4 days post transduction`, `fixed 2 days later` (D95 slice 8);
- **after `by` and `every`**: `recovered by three weeks`, `changed every 3 days`, `every 2–3 days`;
- **before `of` and a noun**, an amount of the noun's stuff (a volume, a mass, an amount of substance,
  a concentration, a duration): `300 μl of CellTiter-Glo`, `0.2 μg/ml of doxycycline`, `24 h of
  puromycin selection`. A percentage before `of` is a proportion (`15% of colon cancers`);
- **as a list with the unit once**: `harvested 4 and 7 days after transduction` (at both), `for 5 or
  10 h`. Not before a noun: `5 and 10 μM etoposide` has no parse — two treatments are two claims (R1).

It cannot be a subject or a verb's object: `The cells contained 4 μg.` has no parse; `The medium
contained 2 μg ml⁻¹ puromycin.` does.

1. **Write the symbol, not the spelled-out unit.** `5 ml`, not `5 millilitres`. The symbol is what
   the unit parser reads; the spelled form is ordinary English words and parses as a different
   thing entirely.
2. **Put a space between the numeral and the symbol.** `37 °C`, `5 ml`, `625 mg`. This is the SI's
   own convention, and it makes the span unambiguous. The exceptions the SI itself makes are `%`
   and the angle symbols (`50%`, `90°`), which close up. A closed-up form (`931g`) is read too, but
   a word with the same spelling competes with it (`5A`, a culture medium, is also 5 A).
3. **Write compound units with a slash, `per`, or negative powers.** `mg/dL`, `mg per kg`,
   `μg ml⁻¹`. One solidus only: `mg/ml/h` is ambiguous without brackets, and the SI says so; write
   `mg ml⁻¹ h⁻¹`. A factor after a space needs its exponent: `μg ml⁻¹`, not `μg ml`.
4. 🔜 **A difference has no consumer yet.** What takes a quantity decides whether it is a value or a
   difference (D95): `at 37 °C` is the temperature 310.15 K, and a difference in °C is read without
   the 273.15 offset, so `5 °C` and `5 K` state the same change. But nothing takes a difference until
   D95 slice 7: `The temperature rose by 5 K.` has no parse. Keep a change as written and record the
   gap.
5. **Write `RCF` for centrifugal force.** `931g` and `931 g` are each read two ways, as the gram and
   as standard gravity, and the reading choice is left open (D93). `931 RCF` is standard gravity
   only. Do not write `931 × g` (`×` is an operator, so the sentence is non-prose) or `931 times
   gravity` (no parse: `at` has no noun-phrase adjunct entry).
6. **One quantity per role.** `kept at 37 °C for 1 h` is fine — two quantities filling two
   different roles. A range is one quantity: write it as rule 8 says, not `between 37 °C and 39 °C`.
7. **A plain value is exact; write a bound out.** `37 °C`, `2 h` and `5 cell lines` state exactly
   that value. Where a bound is meant, say so:

   | Meaning | Write | Also read |
   |---|---|---|
   | at least N | `at least N`, `N or more`, `N or higher` | `≥ N` |
   | at most N | `at most N`, `up to N`, `N or less`, `N or lower` | `≤ N` |
   | more than N | `more than N` | `> N` |
   | less than N | `less than N` | `< N` |
   | about N | `approximately N`, `about N`, `around N`, `roughly N` | `~N`, `≈ N` |

   A bound goes wherever the value goes: `incubated at 37 °C or higher`, `a dose of at least
   5 mg/kg`, `The temperature was less than 37 °C.` A symbol between a noun phrase and a value is a
   comparison without the copula (`the temperature < 37 °C`), but in a sentence of prose write the
   copula and the words. The same forms bound a count (`at least 1,000 cells`), where `fewer than N`
   and `N or fewer` are also read. 🔜 A bound on a difference (`rose by more than 5 °C`) has no
   parse.
   **A proportion** is `N% of`, `half of`, or a bound on either, before a plural noun phrase or a
   mass: `15% of colon cancers`, `more than half of the samples`, `45–60% of the cancers`, `Half of
   the cell pellet was saved.` `none of`, `all of`, `each of`, `most of` and `some of` are read too.
   Write `the` or no determiner before the noun: `such` has no entry.
8. **A range is the pair with the unit once, after it**: `2–3 days`, `80–90% confluence`,
   `30–37 °C`. Both ends are read in that unit and the range means `from … to …`, both ends included.
   Prefer the en-dash; a hyphen with a unit after it (`45-60%`) reads the same. Without a unit a pair
   is not a range — `926-68021` is a catalogue number — and `37 °C–39 °C` or `between 37 °C and
   39 °C` has no parse.
9. **Scientific notation is one number**: `2 × 10⁻¹⁶`, `1.5 × 10³ cells`, `10³`. Write `×` (or `x`)
   and a superscript or caret exponent; a unit after it applies to the whole number
   (`2 × 10⁻³ mg/kg`).
10. **A number word takes a unit as digits do**: `nine days` is `9 days`, for `one` … `ten`. Before a
    noun, join the number to the unit's NAME with a hyphen: `an eight-day viability assay`, `a
    10-minute incubation`. A unit SYMBOL takes no hyphen (`a 2 h incubation`, not `a 2-h incubation`,
    which is read as a word): the SI writes it so, and a hyphen before letters that spell a symbol
    names a compound (`5-mC`, `3-MA`).
11. **An offset goes before its preposition, and `post` is written apart**: `4 days post
    transduction`, not `4 days post-transduction` (one hyphenated token, read as a word). `after`,
    `before`, `post` and `later` take the offset; a bound or approximation on it reads as on any value
    (`about 6 h before collection`).
12. **`per` distributes a count**: `At least 1,000 cells per sample were scored.` reads as a count
    in each sample. A quantity per noun (`at 33 μl per well`) has no parse: write `Each well received
    33 μl of CellTiter-Glo.`
13. **A factor goes before `fewer` or `more` and a counted noun**: `The cell lines contained 0.56-fold
    fewer deletion mutations than typical lineages.` Join the number to `fold` with a hyphen
    (`2-fold`, `two-fold`), and keep the factor as written — do not restate `0.56-fold fewer` as `44%
    fewer` or as a ratio the other way up, since the text fixes no arithmetic and a check reads it as
    written. `a median` before the factor states what it summarises: `a median 0.56-fold fewer deletion
    mutations … compared to typical lineages`. A median is part of the claim, unlike `n` and `P`.
    `compared to` and `compared with` mark the standard as `than` does, after a comparative only.
    Not read: a factor on an adjective (`2-fold more dependent`), before a noun (`a 2-fold increase`)
    or after a verb (`increased 2-fold`), and `10% fewer` or `3 fewer`, which state differences.

**What this does not change.** A statistic is still not a quantity. `n = 37` counts samples and
`P = 4.2 × 10⁻¹³` qualifies an inference; neither is a measured value of a physical quantity, and
both stay out of the claim.

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
adjective (`microsatellite-stable`) *and the lexicon does not carry the term* — if it recurs across
documents, add the multiword entry instead and neither spelling will fork. The D63 hyphen morphology reads it as one
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
import provides them. ⚠ Named *conditions* (`Lynch syndrome`, `MMR deficiency`) also stand bare now —
D70 gave them `lexicon:Num::name`, which grants bare standing without claiming they are mass nouns —
so write them as they appear in prose, without a determiner.

⚠ **A verb the lexicon has only as intransitive has no passive.** WordNet lists both senses of
`incubate` as intransitive, so `The cells were incubated.` has no parse, with or without `at 37 °C`;
`electroporate` is not a WordNet verb at all. Until the lexicon records these verbs' objects, write a
verb it has as transitive: `The cells were kept at 37 °C for 1 h.` parses.

## Worked example (one WRN sentence)

**Original (journal):** *"MSI cancer models required the helicase activity of WRN, but not its
exonuclease activity."*

**Controlled:**
> MSI cancer models required the helicase activity of WRN.
> MSI cancer models did not require the exonuclease activity of WRN.

Two same-shape SVO clauses; the contrast is preserved as an explicit negation; both compose.

## Worked example (one WRN methods sentence)

**Original (journal):** *"Experiments were performed in triplicate by adding the appropriate volume
of lentivirus to integrate vectors that encoded the desired sgRNA and the plates were spun at 931g
for 2 h at 30 °C."*

Two claims, a relative clause, a coordination, and three quantities — one of them the ambiguous `g`.

**Controlled:**
> Experiments were performed in triplicate.
> The vectors encoded the desired sgRNA.
> The plates were spun at 931 RCF for 2 h at 30 °C.

R1 splits the compound sentence; the relative clause becomes its own claim; `931g` becomes `931 RCF`
because the symbol is ambiguous between the gram and standard gravity (rule 5). The two unambiguous
quantities stay as written, filling two roles of one event — which is the case the methods register
produces constantly and the results register almost never does. All three sentences parse; the
third has 6 readings, against 12 with `931g`.

**What is still lost:** *"in triplicate"* is a replication count, which is D52's, not a quantity.
Do not rewrite it as `3 replicates` to make it look like one.

## Each row is a probe

**The guide's claims are asserted by tests, not by the reader's diligence.** The previous posture —
"expected to drift as the grammar grows; check a claim against the baseline before relying on it" —
is how three DO items came to over-claim and the `because` row came to prescribe destroying content
the parser carries. The quantities section already had the better pattern (a corpus plus
`kernel/tests/quantity_corpus.rs`); syntax now has it too.

| probe | what it pins |
|---|---|
| `probe_step5_construction_families` (29 probes, `--ignored`) | the DO/DON'T families from the 5–9 token band: reduced relatives, appositives, comparatives, negative subjects, `rather than`, verbless NPs. Each failing family is paired with the nearest probe that PARSES, which is what localizes it. |
| `probe_step5_families_medium` (39 probes) | the 10–16 token band: zero relative vs `that` relative, zero vs `that` complementizer, `but not` and `rather than` by operand category, NP coordination by position, `both … and`, `without` + gerund, `So` vs `Yet`, imperatives, fronted conditionals. |
| `probe_closed_class_paradigms` (6 paradigms) | every member of the indefinite-pronoun, possessive-determiner, possessive-pronoun, reflexive, subordinator and complementizer series, as OK / WRONGCAT / LEXICAL. A form "present" via a WordNet content entry is NOT coverage — `nothing` passed that way for 15 occurrences while denoting an entity. |
| `probe_reduced_relative_pp` | the passive participial ladder: bare, + adverb, + adjunct PP, + agent by-phrase. |
| `probe_ger_post_nominal` | the ACTIVE participial ladder, each probe paired with its relativizer control, and the SEM printed — the construction once "parsed" as a compound noun pile and a reading count alone did not catch it. |
| `kernel/tests/closed_class_determiners.rs` | the bootstrap-level assertions for what landed `2026-10-13`: the factive subordinators' two proof obligations, `rather than` ≡ `but not`, `unless`'s negated antecedent, the possessive paradigm in both numbers, indefinite quantification, `can not`. |

Three probes carry a tag the guide depends on:

- **`LEX-GAP` is not `GAP`.** A probe whose vocabulary the served chain lacks is tagged separately,
  because a missing word is a lexicon problem and a missing rule is a grammar one. That distinction
  retracted two rows mid-revision: the passive AGENT by-phrase was read as a gap when the real cause
  was a derived `-ly` adverb, and `cannot` was read as a wrong category when it had no entry at all.
- **An OPEN parse counts.** The success criterion below admits closed OR open; a factive subordinator
  and a possessive determiner both yield open parses by design.
- **A parse is not a correct parse.** `nothing` "passed" for 15 occurrences on a noun reading. Where
  a row's claim is semantic, the probe asserts the SEM, not that a parse exists.

## Success criterion

A passage is "parser-faithful" when every sentence yields a **closed or open** kernel-checked parse
(no GRAMMAR-GAP), and the set of parses captures the passage's factual claims. The experiment measures
the closed/open/gap distribution on the rewritten WRN page against the original.

**Quantities have their own corpus.** The CNL page is results prose and contains one unit in 2,738
words; the methods material contains 35 in 4,912. `experiments/parsing/quantities/` holds 36
sentences derived from the WRN paper. Each names the relations its readings must contain, or, for
a gap, the construction it lacks and the D95 slice that owns it; `kernel/tests/quantity_corpus.rs`
checks them without a database, and its README records the full-lexicon run. A quantity gap stays
distinguishable from a syntax gap.
