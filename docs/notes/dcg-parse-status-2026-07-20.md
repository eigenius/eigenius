# DCG parser — status and next steps (2026-07-20)

## Where we are

We're reducing spurious ambiguity in the DCG parser, measured on the first page of the
WRN-helicase paper. Two rules stand above the rest: **coverage is non-negotiable** (every
sentence must parse — grammar-gap 0), and, set this session, **correctness comes before
ambiguity** — a sentence closing on the *right* reading matters more than a low reading count.

## What we did this session

We found and fixed a bug in the sense reranker (the LLM step that chooses which meaning each
word takes). Adjective senses were being shown to the model as a meaningless placeholder
("grammatical function-word reading") instead of their real dictionary definitions — the code
that fetches a definition only looked at the top of the term and missed the definition, which
sits one level deeper for gradable adjectives. With no real definition, and a prompt that says
"omit function words," the model dropped the adjectives and picked medical-qualifier concepts
instead. So "specific" and "stronger" were read as noun-qualifiers, not adjectives, and the
sentences closed on the wrong reading.

The fix teaches the definition lookup to walk into the term and find the gloss. It's
parser-only — no database change, because the definition was already stored. Afterwards the
reranker ranks the adjectives first and the affected sentences close on the correct reading.
Committed as **b757274**, with a variance-checked re-baseline (5 measurement draws; coverage
held in all 5).

One expected side effect: restoring the correct adjective readings *raises* the ambiguity
count, because the broken reranker had been hiding ambiguity by wrongly discarding adjectives.
That's an acceptable trade under "correctness first."

## What we found in the two follow-ups

1. **One sentence explodes to 170 readings** ("Many cancers exhibit an impairment of a DNA
   repair pathway"). This is *benign*: all 170 mean the same thing, differing only in how the
   noun pile "DNA repair pathway" and the "of" phrase bracket, and the intended reading is
   among them. About **half** of its apparent structural count is a **measurement artifact** —
   "cancer" comes from two dictionaries (WordNet and UMLS) that were merged in *meaning* but
   not in their *type label*, so the same reading gets counted twice.

2. **The faithfulness measure is weak.** We gate on the count of single-reading ("encoded")
   sentences, but a sentence can be encoded on the *wrong* reading. The better approach is a
   small, growing set of targeted checks that assert a sentence *contains* the right reading
   (we already have one such check; we'd add more).

Both follow-ups are blocked by the same cross-dictionary labeling artifact: it inflates the
count and would confuse the targeted checks.

## Next steps

1. **Normalize the cross-dictionary artifact** — either a cheap fix in the measurement (ignore
   the dictionary-of-origin label when counting structures) or the deeper fix in the alignment
   (give a merged concept a single type label). The deeper fix also cuts real readings, not
   just the count.
2. **Add correctness "canary" checks**, starting with "specific is an adjective," to guard this
   session's fix from regressing.
3. **Then return to the real structural ambiguity** (the noun pile), now on a clean footing.

## Correction to the step-1 diagnosis (2026-07-20, continued)

Reproduced the x170 sentence deterministically (`trace_one_sentence`, cap-only, no LLM, aligned
snapshot `wordnet-umls-aligned-2026-07-17-chv`): **170 readings / 34 structural skeletons**. The
skeletons pair up — `skel[0..14]` (a slot typed `C§`, a UMLS CUI class) mirror `skel[15..29]`
(typed `n§`, a WordNet class), structurally identical otherwise. So ~half the skeleton count is a
cross-lexicon class-label split, as the note said. **But the mechanism is not what the note claimed.**

The raw IRIs (not sense-erased) show "cancers" (the head, `G#1`) seeds **four** senses: three
WordNet (`n01977832`, `n09752657`, `n14239918`) **plus one UMLS `C1547140`**; "impairment"
similarly seeds three WordNet plus `C0684336`. These C-class senses are **not merged-but-mislabeled
duplicates** — the alignment redefines *both* `cat` and `sem` to WordNet, so a real merge collapses
cleanly. They are **unmerged concepts the adjudicator marked `same:false` on purpose**
(`alignment.jsonl`):

- **`C1547140` "cancer"** — UMLS semantic type **T091 "Biomedical Occupation or Discipline"**,
  sourced from **HL7v2.5** with MTH preferred name **"Specialty Type - cancer"**. A metadata /
  administrative code (the oncology *specialty*), not the disease. **Junk that should not be a
  lexical noun sense at all** — the same class of collision the `drops.json` set exists for, except
  the current drop criterion only catches *case-mangled* atoms (`gENE`→`gene`), and `C1547140`'s
  atom "Cancer" is proper-cased, so it slips through.
- **`C0684336` "impairment"** — semantic type **T033 "Finding"**, "Impaired health", CHV/LNC/AOD
  sourced. A **legitimate distinct clinical sense**; the adjudicator correctly kept it separate. Its
  multiplicity is real sense ambiguity for the reranker to prune, not a lexicon defect.

**Consequences for step 1:**

1. The note's "give the merged concept a single type label" fix rests on a misdiagnosis — there is
   no merged-but-split concept here; there is an **unmerged metadata artifact**. The real structural
   fix is to **drop `C1547140`-class junk** (broaden the drop adjudication beyond case-mangling to
   metadata/administrative CUIs — T091 occupation/discipline + HL7-style specialty codes — that
   collide with a common word already WordNet-covered). Dropping the junk cuts real readings **and**
   deflates the skeleton count honestly (the `C§` skeletons disappear because no C-class cancer sense
   remains to fill the head). Needs a drop pass + reseed.
2. The "cheap measurement fix" (normalize `C§`/`n§` in `erase_senses`) is a **band-aid**: the
   skeleton metric is faithfully reporting real forest content — the junk sense genuinely produces a
   distinct term — so collapsing `C`/`n` would *hide* real junk rather than fix a miscount, and it
   risks conflating WordNet `n`/`v`/`a` POS classes. Rejected under "fix the structure, not the
   measurement."

**Step 2 is not actually blocked.** The correctness canaries assert a sentence *contains* the right
reading; extra junk senses don't confuse a presence check. Exemplars to model new canaries on:
`verify_attributive_comparative_at_scale` and `definite_negation_collapses_referential`
(snapshot-gated, parse-and-assert-a-reading).

## Outcome of the drop fix (measured, 2026-07-20 continued)

Implemented the structural fix: a second drop path in `crates/eigenius-lexicon-align/src/drops.rs`
(metadata-artefact concepts — curated HL7 code-table prefixes + SNOMED `(qualifier value)`/
`(attribute)`/`(qualifier)` tags), regenerated `drops.json` (17 → 275; `C1547140` caught; no
same-surface drop/merge conflict), reseeded, rebuilt the aligned snapshot
(`wordnet-umls-aligned-2026-07-20-metadrops`), measured.

**Coverage holds — grammar-gap 0, missing-lexeme 0** (the non-negotiable gate; 251 of 275 atoms
fired at import, corpus-wide, breaking no parse). The junk is gone: on the x170 sentence
`C1547140` no longer seeds, and the cross-dictionary `C§`/`n§` skeleton split is removed — **34 → 17
skeletons** (cap-only, deterministic), the ~half the note flagged. `C0684336` "Impaired health"
(a real `(finding)`) is correctly kept.

**But the full-page aggregate effect is ~neutral, and this is the real finding.** The drift-free
cap-only A/B (same code, old vs new snapshot) is total-readings **2304 → 2304 (flat)**, total-skeletons
**720 → 709 (−11)**, encoded 4 → 4. The single reranked draw (readings 1328→1266, skeletons 446→433,
encoded 8→9) sits **entirely inside the baseline's own drift bands** — drift, not signal. The cause is
**cap-backfill**: dropping a junk sense frees a `SENSE_CAP=2` slot and the parser refills it with the
next sense, so the per-sentence win does not aggregate. This is the same lesson the WordNet↔UMLS
alignment hit (README §"Result — measured, and negative"): removing a competing sense is *correct and
coverage-safe* but is **not the multiplicity lever** while the cap backfills.

**Verdict:** the drops are worth keeping — they remove genuine lexicon junk (`Specialty Type - cancer`
is not a noun), hold coverage, and close the README's named gap — but the aggregate multiplicity lever
is `SENSE_CAP`/backfill, not lexicon drops. The next real lever is either the cap-backfill discipline
(don't refill a freed slot with a lower-ranked sense) or the noun-pile structure (GH#97), not more
drops.

## Lever 1 (adjective-outside-compound NF) — landed, and the "regressions" diagnosed (2026-07-20)

Committed `a0acb4d`. A `Guard::NotAdjectiveRefined` on both operands of `kind_compound` (and the head
of `named_compound`) forbids a compound over a **gradable-adjective-refined** operand (`is_adjective_refined`
positively matches the `measurements:gt`/`lt` degree axiom, walking the restrictor spine through the
un-reduced `(λx. gt…)(x)` and its hidden `Ann`). Canonical form: `adj*(compound-core(N))`.

**Result (live reranked, on the metadrops snapshot): coverage holds (grammar-gap 0), encoded 8→11,
total-skeletons 446→336 (−25%), readings 1328→1103.** Deterministic cap-only (drift-free): skeletons
720→579 (−20%). `#5` and `#11` reach ENCODED. 1648 kernel tests green incl. the differential packing
oracle.

**The two apparent per-unit regressions are NOT the NF** — proven by a same-`ranks.json` replay A/B
across the commit boundary (identical sense choices, so any delta is the grammar alone):

| unit | pre-NF | post-NF | verdict |
| --- | --- | --- | --- |
| `#9` "We evaluated MSI as a biomarker for WRN dependency" | 31 | 31 | **NF-neutral** — the live 2→31 is pure reranker sense-choice drift between draws, not the NF. |
| `#7` "…dependence on specific repair proteins" | 2 | 18 | NF collapses the adjective correctly (specific outside, 1 bracket); the increase is **cap-backfill**. |

`#7`'s 12 post-NF skeletons decompose into three PRE-EXISTING axes the freed cap slot re-admitted, none
new structure: **modal scope** (`can` in/out of the `on`-PP: `And(Possible(v), prep_on)` vs
`Possible(And(v, prep_on))`), **PP-attachment** (`on … proteins` → dependence vs VP), and the
**cross-lexicon `C§`/`n§` sense pairs** of `proteins`/`repair` (each a WordNet + UMLS sense, miscounted
as distinct skeletons — the same cross-dictionary artifact §"Correction to the step-1 diagnosis"). The
deterministic cap-only per-unit confirms the NF did not inflate `#7` (72→66, slightly down); the reranker
draw's cap budget is what redistributes.

**So the NF is sound and a net win.** `#7`'s residual is the SENSE_CAP/**cap-backfill** lever (the same
one the metadata-drops A/B flagged) plus the already-scoped PP-attachment and cross-lexicon levers — not
an NF defect. `#9` needs nothing. No further NF change indicated.

## N-N modifier NF — grounded, NO target (2026-07-20)

Grounded the proposed "N-N modifier left-branching NF" before building it, and it has **no clean
target**: pure N-N compound piles ALREADY left-branch. `MSI biomarkers` → 1 reading; `genes affect
cancer dependency data sets` → 4 skeletons, all nested/left-branching (the existing
`is_compound_refined` head-guard, D63 §8.13), differing only by `compound`-vs-`compound_kind`
(named-vs-common) and the C§/n§ sense artifact — no flat-vs-nested compound over-generation.

The flat-co-modifier piles seen in `#9` (`And(compound(G0), compound_kind(G0), compound_kind(G0),
is_a(G0))`) are NOT general N-N compounding. Two findings:

- `We evaluated MSI as a biomarker` gaps cap-only (0 readings) — `evaluate` is not in `ESSIVE_VERBS`
  (known residue, compound-pile-collapse §7), so `#9` never parses as a clean essive; its cap-only
  mess is spurious compounds around the un-handled `as`.
- The non-essive `A biomarker for WRN dependency is essential` ALSO explodes (31 readings / 21
  skeletons), so it is not the essive. Its skeletons are a **heterogeneous mix**, no single driver:
  `compound` vs `compound_kind` (a word — `WRN` — read as a NAME `cat_np` vs a common noun), PP-attachment
  (`for WRN dependency`), deep compound piles, the C§/n§ cross-lexicon sense pairs (collapsing C/n cuts
  only 21→18, so NOT the dominant driver), and a **spurious `is_a` predication inside a non-essive NP**
  (`is_a(G0, …)` with no essive verb present — a possible over-generation bug, a concrete lead).

**Conclusion: the clean rule-change structural levers are exhausted for this residual.** Lever 1
(adjective-outside NF) was the one with a clean target and it landed. What remains is sense/category
(named-vs-common — reranker), alignment (cross-lexicon merge — reseed), PP-attachment (selectional
typing / underspecification), and the spurious-`is_a`-in-NP lead. None is an N-N compound NF.

## PP-attachment — a red herring, confirmed on two counts (2026-07-20, new session)

Analysed PP-attachment on the current snapshot (metadrops + adjective NF). A minimal-pair PP ladder
(add one PP at a time; count STRUCTURE with senses AND the `C§`/`n§` class-letter fully erased):

| sentence | distinct structures |
|---|---|
| `We queried dependencies` | 1 |
| `… in cancers` | 1 (adds a sense, not a structure) |
| `… in cancers with MSI` | 2 (+1: `with MSI` → dependencies flat vs cancers nested) |
| `Scientists exploit lethality` | 1 |
| `… for therapeutics` | 4 (+3) |

`with MSI` adds exactly **+1 clean structure** — genuine 2-way attachment. But `for therapeutics`
adds +3, and dumping them shows only 2 are attachment (VP-adjunct vs noun-modifier); the other 2 are
**spurious compounds where `for` becomes a noun**: `for` seeds UMLS **`C0521125` "For (preposition)"**
(type T080 Qualitative Concept, NCI) as a content noun, which the compound rule piles into `[For]
therapeutics` — the same function-word-junk class as `as`=arsenic. Each preposition surface carries a
handful of such UMLS content-noun senses (for:4, in:9, as:10, …).

**Two independent reasons PP-attachment is not a grammar lever:**

1. The apparent PP "explosions" are **not attachment** — they are function-word metadata noun senses
   compounding, and they appear only **cap-only**: the RERANKER eliminates them. Replaying `#6`
   (`…lethality for cancer therapeutics`) and `#10` (`…in cancers with MSI`) with reranker ranks gives
   **2 clean readings each, every one carrying a real `prep_for`/`prep_with`** — no `C0521125`, no
   junk compound. (Deleting the glue-word content senses was already TRIED AND REVERTED — regressed
   coverage 1→5, compound-pile §7 — and the reranker already handles them, so there is nothing to do.)
2. The genuine PP-attachment that survives reranking is **small and REAL**: `#6`/`#10` are 2 readings
   each — `cancers with MSI` vs `dependencies with MSI`, both well-formed logical forms. Nothing
   spurious to collapse with a rule; it is genuine structural ambiguity whose only levers are
   **selectional typing** (types pick the felicitous attachment) or **underspecification** (represent
   the choice) — the shelved PP-attachment note's Lever A / Lever C — NOT an NF/rule change.

**Verdict:** red herring, again. The "PP" residual is (a) cap-only function-word junk the reranker
already removes, and (b) a genuine 2-way ambiguity that is not rule-collapsible. No grammar-rule work
is warranted; if pursued, it is the selectional-typing/underspecification track (bigger scope), not a
combinator guard.

## Three bundled importer fixes — big win, but a coverage regression on #46 (2026-07-20, new session)

Reseeded `wordnet-umls-aligned-2026-07-20-fixes` with three importer/lexicon fixes:
1. **function-word-surface skip** (`eigenius-umls` `is_grammatical_surface` += prepositions/conjunctions)
   — `For (preposition)` `C0521125` and siblings + chemical-symbol homonyms no longer seed content nouns.
2. **`evaluate`/`assess`/`deem` → `ESSIVE_VERBS`** (`eigenius-wordnet`).
3. **informational-metadata drops** (`drops.rs`: INFO-TUI + code/info name, substance-TUI floor) —
   `Protein Info` `C1521746` etc.; drops.json 275 → 397. (The "cross-lexicon merge gap" was a misnomer:
   the `same:false` pairs are metadata junk like this, or GENUINE distinct senses — no real merge gap.)

**Verified working:** `For` fully gone (`#6` cap-only 8 → 1 skeleton), essive parses (`#9` 0/gap → 22).
**Big multiplicity win (reranked):** total-readings **1328 → 967 (−27%)**, total-skeletons **446 → 322
(−28%)**, encoded **8 → 10**.

**But COVERAGE REGRESSED — grammar-gap 0 → 1** (non-negotiable gate FAILS), on `#46` *"Some MSI lines and
some MSS lines were represented by these screening data sets."* — the exact cap-fragile sentence the
compound-pile note reverted a glue-word cut over. Diagnosis:

- Every sub-part and near-variant parses (`…by data sets` 88, generic-`lines …these screening data sets`
  66); only the full three-domain-compound combination gaps.
- NOT beam pressure: gaps at `cell_beam` 512/1024/**2048** and `sense_cap` 16. A clean parse path does
  not exist.
- The only new changes on its words: dropped `C1546701 "Line"` (a specimen-code) and skipped the `by`
  content noun (`C4761448 "Buyei Chinese"`). Its previously-working parse was **relying on one of those
  junk senses** (the compound-pile "propping up cap-fragile parses" mechanism); removing the junk removed
  the derivation, and no clean one exists.

**Disposition (needs a call):** the fixes are correct (they remove genuine junk) and a large win, but
`#46` broke the coverage gate. Options:

(a) back off the tipping drop(s) [reseed, re-masks the gap with
junk]; (b) fix `#46`'s real grammar gap (coordinated-passive × 3 domain compounds — the clean parse
should exist without junk); (c) accept it (the parse was junk-dependent). Not landing until resolved.

## #46's real gap DIAGNOSED — a broad agentive-passive `by` composition bug (2026-07-20)

The junk drop exposed a **pre-existing** grammar gap, not a new one: on the metadrops (pre-fix)
snapshot `Cells were represented by screening data sets` "parsed" only via `C4761448 "Buyei Chinese"`
(the `by`=noun I dropped) forming a spurious transitive compound `represent(cells, [screening data
sets by-noun])` — never a real passive. The clean agentive passive does not compose.

Isolated with a swap-ladder (fixes snapshot). The agentive-passive `by` (`by_agent`,
[closed-class.esl §Agentive passive](../../ontologies/lexicon/closed-class.esl), category
`(S[pass]\NP \ (S[pss]\NP)/NP) / cat_np(Entity, num_any)`) — precise mechanism:

| agent after passive `by` | parses? |
|---|---|
| bare-plural KIND — `by cells`, `by data sets`, `by screening sets` | **yes** |
| determined SIMPLE — `by these sets` | **yes** |
| **determined COMPOUND — `by these data sets`, `by these screening data sets`** | **NO** |
| bare/adj COMPLEX — `by large data sets`, `by screening data` | **NO** |
| pronoun — `by them` | **NO** |

The determined-compound agent is a valid NP everywhere ELSE: it composes as a subject (`These screening
data sets are essential` 24) and as a plain transitive OBJECT (`Cells affect these screening data sets`
48). A determiner produces a **type-raised GQ**, consumed as a prep object by the GQ-as-prep-object
rules (`gq_prep_*`, `combinators.rs`). So the gap is: **`by_agent` + a determined-COMPOUND GQ does not
compose**, though `by_agent` + a determined-SIMPLE GQ (`these sets`) and + a bare-plural kind
(`data sets`) both do, and a normal verb-object slot takes the compound GQ fine. The fix is in the
GQ-as-`by`-agent path — why the compound-refined GQ (`Σ. compound_kind(…)`) fails the `by_agent`
agent slot when a simple-class GQ and the transitive-object slot accept it. A focused grammar fix
(combinator/closed-class + reseed to validate), broader than `#46` but specific.

**FIXED (2026-07-20) — `gq_prep_passive_agent`, a KERNEL rule (no reseed).** Added a fourth
GQ-as-prep-object rule in `kernel/src/dcg/rules/combinators.rs`: the agentive `by`
(`fwd(passive-VP-result, NP_agent)`) now takes a type-raised GQ agent — `λTV. λp. Q(λagent.
by(agent)(TV)(p))`, `by`'s own result category — exactly as the other `gq_prep_*` rules quantify a
preposition's object. `#46` **0 → 120** readings; `by these data sets` 0→6, `by large data sets` 0→9,
`by these screening data sets` 0→48. Trigger-disjoint from the three existing GQ-prep rules; 1648+142
kernel tests green (differential packing oracle holds). Residual (separate, pre-existing): a bare
PRONOUN agent (`by them`) still gaps — pronouns are not raised GQs, a small separate follow-up.

**Full result — all four fixes (3 importer + this grammar rule), reranked, COVERAGE PASS:**
grammar-gap **0**, missing-lexeme 0; encoded **8 → 11 (+3)**; total-readings **1328 → 943 (−29%)**;
total-skeletons **446 → 354 (−21%)**. The multiplicity win lands with coverage held and no junk crutch.

## Re-baseline + high-reading-bucket patterns (2026-07-20, new session)

**Re-baselined** `baseline.json` against `wordnet-umls-aligned-2026-07-20-fixes` (5-draw variance study;
grammar-gap 0 in all 5). Expected = drift-free replay of the encoded-floor draw: **encoded 10** (band
10-11), **total-readings 931** (band 931-986, ceiling 1100), **total-skeletons 326** (band 326-354,
ceiling 400). `eval-parse-rate.sh` scores the floor draw exactly.

**High-reading buckets are SENSE-dominated, not structural** (`less dependent on WRN` 61 readings / **2
skeletons**, sense× 30; coordinations ~10 skeletons / sense× ~12). The sense product comes from:

1. **Genuine WordNet polysemy** — `lines` = line/occupation/cell-line; `group` = general/social.
2. **UMLS qualifier concepts colliding with ADJECTIVES** — `rare` seeds the adjective PLUS `C0522498`
   "Rare" (Qualitative) + `C0521114` "Infrequent" (Temporal); `indeterminate` → `C0205258`. These are
   **invisible to the drop/merge pipeline**: a candidate requires a WordNet **noun** collision, and an
   adjective-colliding qualifier is never a candidate (`C0522498`/`C0521114` = 0 in candidates/drops/
   merges). The importer still seeds them as content nouns.
3. **Residual function-word reifications** — `C1550557` "RelationshipConjunction - and" seeds `and`;
   `and` is not yet in the function-word skip.

**Correction to the earlier claim:** genuine cross-lexicon *twins* are mostly already merged (38k) — the
residual noun pairs (`group`/`Social group`) are genuine distinct senses the adjudicator kept, NOT missed
merges. So the next SENSE levers are (a) an importer-side **adjective-competing-qualifier filter** (skip a
Qualitative/Temporal/Spatial "qualifier"-typed UMLS concept whose surface is a WordNet ADJECTIVE), and (b)
extend the function-word skip to conjunctions/determiners (`and`, …). The residual STRUCTURAL lever is
**coordination scope** (distribution-vs-single-NP + the-referential-vs-kind on the 3-4-way `X, Y and Z`
units). NOT more compound/PP NF work.

## Process note

These are not just edits — each needs me to run cycles of:

- **Builds** — Rust compiles, ~30 s to a minute each.
- **Measurements** over the WRN page. With the live reranker these are non-deterministic, so a
  trustworthy read needs several draws (~5 min each) plus a re-baseline.
- **A reseed** for the deeper alignment fix — rebuilding the 2.8 GB WordNet+UMLS snapshot from
  scratch (hours), after which the baseline must be re-measured.

So the measurement-side fixes can land quickly; the alignment fix is a multi-step,
reseed-and-re-baseline job.
