# Plan — making the CNL parse at the operational beam (GH#97 follow-on)

**Status:** Plan (grounded). Tracks [#97](https://github.com/eigenius/eigenius/issues/97).
**Motivation:** the witnessed chart-cell analysis in
[d63-parsing-scale-and-pruning.md §4a](d63-parsing-scale-and-pruning.md) showed that, for the WRN CNL
corpus, the chart explosion is **nominal sense-product** (the compound rule enumerating WordNet×UMLS
senses per noun) + **function-word noun-sense noise**, *not* verb-argument polysemy — so GH#93
selectional restrictions are off this corpus's critical path. The measured levers, in order:

1. **Contextual LLM sense reranker** — validated: recovers S1 at the page beam (GAP→open×80). The
   deterministic "closed-class-wins" alternative was tried and reverted (harmful: can't distinguish
   `be`-verb from beryllium).
2. **Nominal-modification residual** — the bracketing normal form already exists; the real residual is
   narrower (dual-POS modifiers + bare-NP shift fan-out). **Measure-first.**
3. **Compound-as-preposition-object gap** (S4) — a localized category mismatch.

The five reference sentences (CNL v2 first page) and their current page-beam (64) status:
S1 GAP·S2 open·S3 GAP·S4 GAP·S5 GAP; at a wide beam (1024) S1/S2/S3/S5 parse, S4 does not.

---

## Lever 1 — Configure the serving parse path (cap + beam + injected lemmatizer + opt-in LLM)

**Status: IMPLEMENTED (2026-06-30).** `ParseConfig` (lemmatizer + cap + beam + ranker flag) added in
[server/parse.rs](kernel/src/server/parse.rs); held by `EigeniusService` with a `with_parse_config`
builder ([server/mod.rs](kernel/src/server/mod.rs)); the RPC handler builds the per-request index with
cap+beam, the injected lemmatizer, and the opt-in `allms` reranker (widen-on-failure backstop already
in `parse_scoped`). Threaded through `start_server`
([server/lifecycle.rs](kernel/src/server/lifecycle.rs)). CLI: `serve --morphy-dict` (env
`EIGENIUS_MORPHY_DICT`, default in-repo dict) + `build_parse_config` load Morphy (graceful fallback to
`Identity`); same config reused by local `lexicon parse`; `eigenius-wordnet` dep + `allms` feature
passthrough added ([cli/Cargo.toml](cli/Cargo.toml)). Defaults: cap=2, beam=64, `Identity` until a
binary injects Morphy, ranker off (on iff built `--features allms`). Verified: kernel 1595 lib +
100 determiner tests green; clippy clean (default & allms); runtime smoke confirms Morphy loads.
*Deferred:* a dedicated unit test that a mock mis-ranking ranker is recovered by widen-on-failure (the
logic exists and is exercised by the DB-backed measurements; a focused mock test is a follow-up).

**The gap.** Both serving entry points build a **bare** `LexicalIndex` — no sense cap, no cell beam,
no ranker — and use the **`Identity` (no-op) lemmatizer**, so they neither defend against the
full-lexicon OOM nor reduce `events→event`/`is→be`:
- server RPC: [kernel/src/server/parse.rs:72](kernel/src/server/parse.rs#L72)
- CLI local: [cli/src/main.rs:1858](cli/src/main.rs#L1858)

The test harness already has the right config — mirror it:
[db_backed_encoding.rs `build_index`](crates/eigenius-wordnet/tests/db_backed_encoding.rs#L131)
(`SENSE_CAP=2`, `CELL_BEAM=64`, ranker under `allms`).

**Architecture decision (settled): inject the lemmatizer, do NOT relocate Morphy.** The `Lemmatizer`
trait already lives in the kernel ([dcg/lemmatizer.rs:36](kernel/src/dcg/lemmatizer.rs#L36)) with
`Identity` as the default impl ([:42](kernel/src/dcg/lemmatizer.rs#L42)). `MorphyLemmatizer`
([crates/eigenius-wordnet/src/lemmatizer.rs:32](crates/eigenius-wordnet/src/lemmatizer.rs#L32)) is
parameterized by WordNet data (`*.exc` exception lists + a `data.{noun,verb,adj}` lemma-membership
oracle, [:44](crates/eigenius-wordnet/src/lemmatizer.rs#L44)) and parses WordNet's file format — so it
belongs in `eigenius-wordnet`. The kernel **cannot** import it (wordnet→kernel already; importing back
cycles). Resolution: the server holds a configurable `Box<dyn Lemmatizer>`; the top-level binary (which
may depend on wordnet) wires Morphy in. Kernel keeps only the trait; WordNet data stays out of the
kernel.

### Tasks
1. **A parse-config struct** carrying `sense_cap: Option<usize>`, `cell_beam: Option<usize>`, and an
   optional ranker toggle. Thread it to where the served index is built ([parse.rs:72](kernel/src/server/parse.rs#L72)).
   Defaults: cap + beam **on** (the OOM defense the serving path lacks today); ranker **off** (keeps the
   server deterministic by default).
2. **Make the server lemmatizer injectable.** Replace the hardcoded `Identity` at
   [parse.rs:73](kernel/src/server/parse.rs#L73) (and the CLI at [main.rs:1859](cli/src/main.rs#L1859))
   with a held `Box<dyn Lemmatizer>`; default `Identity`, set to `MorphyLemmatizer::load(dict)` from the
   binary. **Config decision (settled):** the dict path is a **CLI option** for now (the serve/CLI
   binary depends on `eigenius-wordnet`, loads Morphy, and injects it); **eventually this moves to the
   orchestrator** (lemmatizer/lexicon provisioning owned there). So the kernel server stays
   lemmatizer-agnostic (trait only) at every step — only *who supplies the dict path* migrates
   CLI-flag → orchestrator.
3. **Wire the cap + beam** onto the built index: `.with_sense_cap(n)`
   ([lookup.rs:342](kernel/src/dcg/lookup.rs#L342)) `.with_cell_beam(m)`
   ([:352](kernel/src/dcg/lookup.rs#L352)).
4. **Opt-in LLM reranker** under `allms` + `ANTHROPIC_API_KEY`: `.with_sense_ranker(Box::new(r))`
   ([:371](kernel/src/dcg/lookup.rs#L371)) with `AnthropicSenseRanker::from_env()`
   ([sense_ranker.rs:104](kernel/src/dcg/sense_ranker.rs#L104)). Same `#[cfg(feature="allms")]` pattern
   the harness uses.
5. **Completeness backstop is already present** — `parse_scoped_open`'s widen-on-failure
   ([lookup.rs:926](kernel/src/dcg/lookup.rs#L926)) re-admits cap-dropped senses, so an LLM mis-rank
   costs a re-parse, never a lost parse (proposer-behind-oracle, D64). No new code; add a test that a
   wrongly-down-ranked sense is recovered.
6. **Cost story.** One reranker call/sentence (`contextual_sense_ranks`,
   [:974](kernel/src/dcg/lookup.rs#L974)) is fine interactively; for batch encoding add a sense-rank
   cache or accept latency. Document the non-determinism (acceptable — kernel gates validity).

### Acceptance
- Server/CLI parse a full-lexicon sentence without OOM (cap + beam active) — they currently can't.
- With `--features allms`, S1 parses at the page beam (matches the harness A/B).
- Closed-term grammar battery stays green; cap-only path remains byte-deterministic.

### Lands in
[kernel/src/server/parse.rs](kernel/src/server/parse.rs#L72) ·
[cli/src/main.rs](cli/src/main.rs#L1858) · a new parse-config (server module) ·
binary startup wiring (Morphy injection).

---

## Lever 2 — Nominal-modification residual (measure-first; the bracketing NF already exists)

**Status: PARTIAL (2026-06-30) — adaptive beam-widen landed (2/5 → 4/5); S4 needs structural work.**
Measured the post-all-fixes page-beam coverage: the LLM alone got 2/5 (S1,S2); the residual was
beam-limited (deterministic beam sweep: S2 b64, S3 b128, S1/S5 b256, S4 not even at b1024). Rather than
a flat beam bump (which would re-OOM long sentences — why beam=64 exists), added **beam
widen-on-failure** (`CELL_BEAM_WIDEN_MAX=512` in `lookup.rs`): `parse_scoped_open` escalates the cell
beam alongside the sense cap for a known sentence that gaps, so beam-limited short sentences recover
while the base beam stays the long-sentence OOM defense. Result: **4/5 parse deterministically**
(S1 open×178, S2 open×38, S3 open×180, S5 open×80); the LLM tightens forests but doesn't change
coverage. Regression-safe (battery 103 + widen tests green; bare/cap-only indices don't widen).

**S4 detailed analysis (`Scientists can exploit synthetic lethality for cancer therapeutics.`) — the
lone holdout.** It is grammar-complete on the small lexicon (modal + prep-object + compound all parse),
yet gaps even at beam 1024 + LLM. Cause, witnessed in the full-span cell `cell[0..7]`: it is dominated
by the **whole-sentence noun-pile reading** — `cat_n(Σ_, …)` refined nouns, **685k items at cap=16**
(and at cap=2 the top cell is `shapes=1 cat_n(Σ_,sg)` — *only* the noun pile, no `cat_s` at all). Every
token carries a noun sense the N-N-compound + attributive-adjective rules chain into one giant refined
noun: `scientists`(n) · **`can`(n=container)** · **`exploit`(n=feat)** · `synthetic`(n/adj) ·
`lethality`(n) · **`for`**(noun noise) · `cancer`(n) · `therapeutics`(n). Unlike S1/S3/S5 — where
non-nominal tokens (`is`/`are`/`does`/`between`/`two`/`each`) break the chain into sub-spans — S4 has
**no chain-breaker** (`can`/`exploit`/`for` all carry noun senses), so the pile spans all 8 tokens in a
Catalan-bracketing × sense-product explosion (685k) that crowds the intended `cat_s` reading out of the
forest/felicity budget (`DEFAULT_FOREST_CAP`). Neither the sense cap, the beam, nor the LLM
sense-reranker resolves it — it's structural.

**Cross-POS prune experiment (2026-06-30, flag-gated `with_pos_prune`, off by default).** Prototyped
the seed-time cross-POS prune: a surface with a closed-class (grammatical) reading drops its open-class
**nominal** (`cat_n`/`cat_np`) readings — the compound-rule noise (`can`→container, `for`→noun,
`is`→beryllium) — while keeping open-class **verb/adj** (so `is`→`be`-verb survives, the case blanket
closed-class-wins wrongly killed). Test-run result (page beam + adaptive widen, full lexicon):

| | default (no prune) | + cross-POS prune |
|---|---|---|
| S1 | open×183 (**59.5s**) | open×256 (**0.9s**) |
| S2 | open×212 | open×16 |
| S3 | open×192 | **GAP (over-pruned)** |
| S4 | **GAP** | **open×256** ✓ |
| S5 | open×210 | open×208 |

Findings: the prune **cracks S4** (the noun-pile holdout) and gives a **~60× speedup** (the pile noise
is gone), and `is`→`be`-verb is preserved — strong evidence it's the right lever for the noun-pile.

**S3 "regression" investigated — the prune is CORRECT (not a regression).** Localized the S3 gap to the
**do-support + to-PP** case (`does not lead to cell death`; do-support *transitive* — `WRN does not
affect cells` — is prune-safe, CLOSED×1). Reading the sems settled it:
- `WRN does not affect cells` → `affect(WRN, cells) → False` — the **real** negation.
- `WRN does not lead to cell death` → `$quant$(Σ. compound_kind(…, Σ. compound_kind(…)), λ. vNNN_t(…))`
  — a **noun-pile** ("lead to cell death" chained as nouns), deferred quantifier, **no negation** —
  junk. The full-S3 `open×192` (without prune) has the same noun-pile shape.

So S3 had **no real parse** with or without prune; the without-prune `open×192` was junk, and the prune
removed it instead of masking the real gap behind it. The genuine blocker is structural: the VP-adjunct
prep `to` is **finite-mood-locked** (`cat_s(dcl, fin)` in its category), so `lead to cell death` cannot
form as a **base** VP under do-support (`does not [lead to cell death]`), and the real `¬(lead→death)`
reading never assembles — only the noun-pile does. **This is the same mood-lock behind S4's modal+PP.**

**⇒ The next fix (helps S3 *and* S4 robustly): make the VP-adjunct prep mood-polymorphic** (accept a
base VP as well as finite), so PPs attach under do-support/modals and the real reading forms — which,
being verb/prep-based, then survives the prune. A Lever-3 follow-on on the prep category (closed-class
bootstrap → reseed).

**IMPLEMENTED (2026-06-30).** Changed all 8 VP-adjunct prep cats (`to`/`for`/`in`/`with`/`on`/`from`/
`between`/`within`) in `closed-class.esl` from `cat_s(dcl, fin)` to `cat_s(dcl, fin_any)` (the
underspecified mood that `feat_meets` accepts against `fin` and `bse`). Verified on the small lexicon
(regression test `vp_adjunct_pp_attaches_inside_a_base_vp`): the PP now attaches INSIDE the base VP with
the **correct scope** — `HeLa can affect BRCA1 to HeLa` → `Possible(And(affects(…), prep_to(…)))` (PP
under the modal), `HeLa does not affect BRCA1 to HeLa` → `And(affects(…), prep_to(…)) → False` (PP under
the negation) — real verb+prep readings, not noun-piles. Battery 104 green, clippy clean.

**VALIDATED on the full lexicon (reseeded 2026-06-30) — 5/5.** With mood-poly + the cross-POS prune, all
five CNL sentences parse at the page beam (adaptive widen), fast: S1 open×256 (1.1s), S2 open×16 (0.9s),
**S3 open×232 (4.6s)**, **S4 open×256 (0.6s)**, S5 open×232 (0.9s) — vs the prior 4/5-at-best and 17–60s
times. S3's reading is now real: `… And(…, prep_to(…)) → False …` (the `to`-PP inside the negation),
not the negation-less noun-pile. The intransitive `lead`+to-PP reading surfaces (`v…_i` + `prep_to`)
where before only the transitive mis-parse did.

**Robustness fix (mood-poly surfaced a crash).** A spurious candidate for a **named-individual subject
+ do-support/modal + PP** (e.g. `WRN does not lead to cell death`) builds a stuck application (the WRN
resource applied as a function); the felicity gate's `readback_val` **panicked** (`apply failed`) — a
parser crash, live on the reseed (the WRN page would hit it). Fixed by making the felicity readback
**total**: `felicity_readback` wraps `readback_val` in `catch_unwind`, so a malformed candidate is
**rejected** (not felicitous) instead of crashing — restoring the totality `eval` already had
(`.ok()?`). The oracle must never panic on an untrusted chart candidate. (A fully fallible
`readback_val` is the cleaner follow-up; the caught panic may still print to stderr.) Regression test
`vp_adjunct_pp_attaches_inside_a_base_vp`; battery 104 green; the previously-crashing probe now passes. With that, the cross-POS prune looks like a near-pure win (cracks S4, ~60× faster,
removes junk) rather than a trade. Also landed this session: **beam-first widen** (grow the beam at a
low cap before widening the cap — raising the cap re-crowds the chart and beams out the constituent a
wider beam was meant to keep).

**Noun-pile analysis of the CNL v2 residual (2026-06-30) — the 7 worst.** The slowest/biggest-pile
units are all long, content-noun-compound-dense, and all GRAMMAR-GAP: unit 47 (21 tok) **565s, 5.3M
items dropped**; unit 38 (16 tok) 244s; unit 57 (13 tok) 219s; unit 54 (16 tok) 155s; units 46/60/61
36–47s. Witnessed on unit 32 (`Some cancers do not respond to immune checkpoint blockade`): the
full-span cell is **32,424 / 34,472 items `cat_n(Σ_, sg)`** — the whole sentence read as a compound
noun. Three properties, all witnessed: (1) **prune-resistant** — persists with the cross-POS prune ON,
because the chain links are *content* nouns (`cell lines`, `data sets`, `deletion mutations`,
`microsatellite regions`, `WRN dependency`, `MMR deficiency`, `mutation phenotype`, `MSI/MSS cell
lines`), not function words; (2) **cap-growing** — 5,610 → 20,456 → 34,472 as the sense cap widens 2→4→8
(the per-noun Cartesian product); (3) **length-catastrophic** — O(n²) cells × the product → millions at
14–21 tokens. Comparatives/relatives/coordination don't *cause* it; they add the length/ambiguity that
makes it worse. (These sentences are also multiply-blocked — most carry `MSI` (alias gap) and/or a
comparative — so taming the pile removes the 565s/OOM risk and lets the real parse survive the beam, but
the gap fixes are still needed for them to *parse*.)

**Fix #1 landed — compound-depth cost PENALTY (ranking; partial).** Each nominal-modification step
(N-N/named/PP compound + attributive-adj) now carries a `Cost` penalty (`Combinator::Compound` +
`COMPOUND_STEP_PENALTY=8`), summed by the combinators, so a deep pile ranks below the shallow correct
parse. Runtime change (no reseed). **Measured (prune on):** it **recovers parses** — unit 60 GAP→open
(40s→13s), unit 39 parses — and the 5/5 first-5 still hold, battery 104 green, no regression. **But it
does NOT fix construction time**: the penalty re-ranks *after* the chart is built, so the longest
sentences still explode in *construction* (unit 57 219→129s GAP, unit 38 244→191s GAP, unit 47 still
>500s). Time is dominated by *building* millions of compound items × the adaptive-widen re-parses.

**Fix #2 TRIED — compound-depth CAP (construction) — REFUTED by A/B (2026-06-30).** Implemented a
`MAX_COMPOUND_MODS = 4` construction-time cap (refuse to form an N-N/named compound past 4
modification steps, counting `compound`/`compound_kind` nodes off the category). Cap FIRES
deterministically on a synthetic deep pile (kernel test `compound_depth_is_capped_at_construction`:
a 5-noun compound parses, a 6-noun compound is refused). **But an A/B on the real corpus (same
binary, `MAX_COMPOUND_MODS` = 4 vs 4096, snapshot `wordnet-umls-2026-06-30`, prune on) showed ~0
benefit — even slightly negative** (the per-attempt `compound_mod_count` walk):

| pile sentence | cap ON (4) | cap OFF (4096) |
|---|---|---|
| unit 32 "Some cancers do not respond…" | GAP 6.3s | GAP 6.2s |
| "We analysed these data sets…" | GAP 199.6s | GAP 183.3s |
| unit 47 "…DRIVE identified WRN…" (21 tok) | GAP 285.5s | GAP 275.6s |

Max cell population **32,176 items, identical both ways**. **The depth hypothesis is wrong for this
corpus.** Anatomy of unit 32's full-span cell (`cell[0..8]`, PARSE_DEBUG): **`cat_n(Σ_,sg) × 30,128`
= 94% is ONE cat-shape** (the whole clause as a single compound noun), differing only in **type
indices (senses)** — a Cartesian sense-product, not Σ-depth. It grows **147 → 4,374 → 18,340 →
32,176 as the SENSE cap widens 2→4→8→16**, and the sentence still GAPs: the **adaptive
widen-on-failure is escalating the sense cap to 16 on a sentence that gaps for STRUCTURAL reasons**,
rebuilding a bigger pile each pass. The cap was *live* (`cap=Some(4)`) in that run and the pile still
hit 30,128 — because the chain also mixes attributive adjectives / PP-modifiers (not counted) and,
decisively, the cost is sense-product *within one shape*, which no depth bound touches. **Fix #2
REVERTED** (kept only as this recorded negative result + the synthetic cap test's insight).

**Fix #2′ — the ACTUAL levers (from the A/B anatomy).** Two, neither depth:
1. **Shape-aware cell beam** (= the GH#93 type-narrowing lever): the beam keeps lowest-cost items
   regardless of shape, so 1024 near-identical `cat_n(Σ_,sg)` pile senses crowd out the real `cat_s`
   reading. Keep **top-K per distinct `cat_shape`** (indices erased) so the pile shape is bounded and
   the real clause survives — collapses the 30,128→~K without losing the parse.
2. **Halt the widen when the top cell has no `cat_s`.** Escalating the sense cap 2→16 cannot fix a
   *structural* gap; it only rebuilds the pile. Detect "top-span cell is all `cat_n`/no sentence
   shape" and stop widening (the sentence is grammar-blocked, not sense-blocked). Cuts the doomed
   re-parses that dominate the long-sentence wall-clock.

**Fix #2″ — the TRUE ROOT CAUSE (dump-verified 2026-06-30): multi-word UMLS stop-word entries defeat
pos_prune.** Dumping the full-span cell (`EIGENIUS_DUMP_CELL=0..8`, cap-only, NO LLM) showed **51,060
top-level `cat_n(…)` items** (vs ~5,285 clause fragments) — the whole clause folded into one refined
common noun. Root cause, traced to a leaf: the span `"do not"` seeds `cat_n(C3840725, sg)` where UMLS
**`C3840725` = "Do not"** (semantic type T033 *Finding*) — a genuine multi-word UMLS lexical entry.
`pos_prune` ([lookup.rs:645](kernel/src/dcg/lookup.rs#L645)) drops a nominal reading only when the
**surface** carries a closed-class entry; `"do"`/`"not"` each do (their single-token noun readings
ARE pruned), but the 2-token surface `"do not"` does **not**, so `surface_is_function = false` and the
CUI noun reading survives at cost 0. So even the `do not respond to` region has surviving nouns, and
with the content-word nouns every token is nominal → the compound/attributive rules bridge the whole
span; the per-token sense product (cap 2 → 16) → 51k. **This is the highest-leverage, most surgical
fix** — narrower than the beam/widen levers above, which only *tolerate* the pile:
- **(prune)** extend `pos_prune` to drop a `cat_n`/`cat_np` reading whose entire surface is composed of
  grammatical function words (a multi-token surface all of whose tokens have a closed-class entry) —
  breaks the pile chain at every function-word bridge; or
- **(import hygiene)** don't emit a content-noun `cat_n` for a UMLS concept whose form is a stop-word
  / stop-word phrase (`"Do not"`, `"to"`, …) — removes the junk entries at the source.
Either breaks the whole-span pile at construction (the real Fix #2 goal) by cutting the *bridges*,
not by capping depth or beaming the aftermath.

**Fix #2″ IMPLEMENTED + measured (2026-06-30) — correct, no-regression, but NARROW.** Extended
`pos_prune` ([lookup.rs:645](kernel/src/dcg/lookup.rs#L645)): a surface counts as a function word (⇒
drop its `cat_n`/`cat_np` reading) if it itself has a closed-class entry **OR it is a multi-token
surface every token of which does** — so the bilexical UMLS "Do not" concept is now pruned. Direct
witness (`EIGENIUS_DUMP_CELL=2..3`): `cat_n(C3840725)` **gone**, only the correct do-support reading
survives; unit 32's full-span cell population **32,176 → 1,962 (16×)**. Battery **104 green**; the
first-7 CNL sems all still parse with identical counts (no over-prune of a real multi-word noun).
**But the corpus-wide A/B (7 pile sentences, prune on) shows it helps only the one sentence with a
stop-word bridge:**

| # | sentence | before | after |
|---|---|---|---|
| 1 | "Some cancers do not respond…" (has "do not" bridge) | GAP 6.3s | **GAP 3.2s** |
| 2 | "Project Achilles screened…" | open×256 15.3s | open×256 14.9s |
| 4 | "WRN dependency may require…" | GAP 45.2s | GAP 45.9s |
| 6 | "We analysed these data sets…" | GAP 199.6s | GAP 182.5s |
| 7 | unit 47 "…DRIVE identified WRN…" | GAP 285.5s | GAP 278.2s |

Sentences 2–7 are **content-noun** piles (`cell lines`, `data sets`, `MSI cell lines` — real words,
no function-word bridge), so this prune cannot touch them (they still drop 500k+ items). **Verdict:**
KEEP the fix (removes genuine lexicon junk — a UMLS *Finding* "Do not" read as a content noun is
always wrong — cheap, no regression), but it is a *targeted* fix for the stop-word-bridge subclass,
NOT a general pile cure. The residual content-noun piles need Fix #2′ (shape-aware beam / halt-widen)
— that lever is still required for the long GAP sentences whose cost is legitimate-word sense-product.

**The LLM reranker CANNOT fix this — tested 2026-06-30 (`--features allms`, live
`AnthropicSenseRanker`).** Hypothesis: cross-POS ranking would demote the junk. Refuted, witnessed:
with the reranker live (39.7s vs ~6s cap-only — API calls confirmed), `cat_n(C3840725)` "Do not"
**still present**, max cell population **32,176 — identical** to cap-only, outcome still GAP.
Root reason is structural, in the reranker's own gate ([lookup.rs:1073](kernel/src/dcg/lookup.rs#L1073)
`if senses.len() > cap`): the ranker is only handed spans with **more than `cap` (=2) competing
senses** (its job is to reorder which survive truncation). The "do not" span has **exactly one**
candidate, so it is never sent to the LLM, and ranking-then-truncate never drops a lone reading. So
cross-POS ranking removes junk only for a *content word with >2 competing POS senses* — never a
**sole multi-word UMLS stop-word entry**. Confirms the fix must be the prune / import-hygiene lever,
not ranking.

**The S4 structural fix (future Lever 2 work):** stop the compound/adjective rules from chaining across
tokens that have a **grammatical (closed-class) or verbal** role — a *targeted* guard on the compound
rule (not the reverted blanket closed-class-wins, which wrongly dropped needed open-class senses like
the `be`-verb) — and/or **cost-penalize compound/refinement depth** so the `cat_s` reading outranks the
deep noun-piles within the forest cap. This is the genuine nominal-residual reduction; the dual-POS /
shift-fan-out items below are part of the same explosion.

**Follow-up — multi-word noun (MWE) handling (surfaced 2026-06-30, pretty-print review).** The
first-7 pretty-print exposed an MWE inconsistency: the term of art *"synthetic lethality"*
(UMLS `C4280020`, a single named genetic interaction) is encoded **two different ways across the
same 7 sentences** depending on cost pressure — in **S4** as the multi-word UMLS entry
(`ΣG1. compound_kind(G1, ΣG2:C4280020…)`), but in **S1** decomposed compositionally into WordNet
*lethality* (`n04791081`) refined by the everyday "man-made/artificial" sense of *synthetic*
(`gt(deg_a01573568(x), std_a01573568)`, adj synset `{man-made, semisynthetic, synthetic}`) —
which is *not* what the term means. Root cause: an available multi-word-expression entry does not
**out-prefer** the compositional noun+adjective reading, so the lexicalized reading loses on
`Cost` in some spans and wins in others. Fix direction (a Lever-2 sibling, not the compound
CAP): a **cost preference for a covering MWE entry** over the token-wise compositional reading of
the same span (longest-match / lexicalized-term bonus), so a registered term of art is encoded
consistently by its CUI. Needs measurement first (how many CNL units carry a term-of-art that
also decomposes). Distinct from the noun-pile explosion (that is *spurious* content-noun
chaining; this is a *correct-vs-idiom* sense choice).

### Original framing (still valid background)

**Correction from grounding.** Canonical bracketing is *already* enforced:
- N-N compounds: **left-branching NF** — a compound's head may not itself be a compound
  ([parser.rs:412](kernel/src/dcg/parser.rs#L412)), so `[[DNA repair] processes]` is the single
  bracketing.
- Stacked attributive adjectives: forced into a **flat Σ** conjunction over the base
  ([:362](kernel/src/dcg/parser.rs#L362)), no nesting ambiguity.

So "add a bracketing normal form" is **not** the work. After Lever 1 removes the per-noun sense product,
the residual structural multiplicity is:
- **Dual-POS modifiers**: a word that is both adjective and noun (`synthetic`, `genetic`) fires *both*
  the attributive rule ([:362](kernel/src/dcg/parser.rs#L362)) and the N-N kind-compound rule
  ([:429](kernel/src/dcg/parser.rs#L429)) — two derivations per such word.
- **named-entity vs kind compound**: a left modifier that is both `cat_np` and `cat_n` fires both
  ([:419](kernel/src/dcg/parser.rs#L419) and [:429](kernel/src/dcg/parser.rs#L429)).
- **bare-NP shift fan-out**: each refined noun spawns plain + plural + mass argument NPs at the
  composed-cell shift ([lookup.rs:1446](kernel/src/dcg/lookup.rs#L1446)).

### Tasks
1. **Measure the post-LLM residual first.** Re-run the cell analysis with the reranker on and the
   wide beam, via the existing diagnostics
   ([`analyze_chart_cells_first_five`](crates/eigenius-wordnet/tests/db_backed_encoding.rs)) — quantify
   which of the three above dominates the surviving `cat_n(Σ_, …)` population. Do not write code before
   this.
2. **Target the dominant one** with a surgical policy/cost (not a new rule):
   - dual-POS modifier → prefer one modification rule, or cost-penalize the rarer (so the beam keeps the
     canonical reading) — a `Cost` bump at the rule site in [parser.rs](kernel/src/dcg/parser.rs#L429);
   - or collapse the named-entity/kind-compound double when both fire.
3. Re-measure: S3/S5 should reach the page beam once the residual is thinned.

### Acceptance
- S3 and S5 parse at the page beam (64) with the reranker on.
- No closed-term regression; the canonical reading is the one kept.

### Lands in
[kernel/src/dcg/parser.rs](kernel/src/dcg/parser.rs#L429) (modifier-rule cost/policy) and possibly the
composed-cell shift [lookup.rs:1446](kernel/src/dcg/lookup.rs#L1446) — **scoped after measurement.**

---

## Lever 3 — Compound-as-preposition-object gap (S4)

**Status: IMPLEMENTED (2026-06-30).** The small-lexicon repro showed the gap is **not**
compound-specific: it's the **VP-adjunct preposition** (`to`/`for` = `(S\NP)\(S\NP)/NP`) object slot,
which only accepted a bare NAME — `to a gene`, `to a gene cell line`, `to gene genes` ALL gapped, while
the noun-modifier `within` accepted every object kind. Root cause: the GQ-as-preposition-object raise
([parser.rs](kernel/src/dcg/parser.rs#L460)) was **restricted to the `cat_pp` functor**. Fix: extended
the raise to the VP-adjunct functor (`bwd(VP,VP)/NP`) with the narrow-scope sem `λV.λs. Q(λx.
prep(x)(V)(s))` (valid because the VP conjunct `V(s)` is independent of the object `x`). Regression
test `vp_adjunct_preposition_takes_quantified_and_compound_objects`
([closed_class_determiners.rs](kernel/tests/closed_class_determiners.rs)); battery 100 + dcg 14 green,
clippy clean. Full-lexicon payoff witnessed: `scientists exploit synthetic lethality for cancer
therapeutics` GAP → **open×72**.

**S4 is now grammar-complete (correction).** An initial read that the full S4 still gapped on a "modal
+ base-verb" grammar issue was a **measurement artifact** — the CLI prints `1 parse` (singular) for a
single parse, and a counting regex that matched only `parses` (plural) reported those as `0`. On the
clean lexicon **every S4 construction parses**, including the modal+PP combos (regression test
`modal_clause_takes_a_vp_adjunct_pp`: `HeLa can affect BRCA1`, `HeLa can affect a gene`, `HeLa can
affect BRCA1 to a gene` all CLOSED). So after Lever 3 **all five sentences are grammar-complete**; S4's
full-lexicon gap is **beam/sense scale**, uniform with S1/S3/S5 — addressed by Levers 1 (LLM reranker)
and 2 (nominal residual), not by any further grammar work.

---

### Original diagnosis (superseded by the above)

**Witnessed asymmetry.** A composed compound NP feeds a **verb** object (`genes affect cancer
therapeutics` → open×36) but not a **preposition** object (`… for cancer therapeutics` → GAP even at a
wide beam), while a single-noun prep object works (`… for therapies`). So a feature/shape mismatch
between the VP-adjunct prep's `/NP` slot and the composed-compound deferred-quant NP.

### Tasks
1. **Reproduce on the small lexicon** (a kernel test, no sense/beam noise): a 2-noun compound as a prep
   object vs a verb object. Gaps there ⇒ real grammar gap; parses ⇒ it was beam.
2. **Diff the slots**: `pretty_term` the VP-adjunct prep object slot (the prep lexical cat —
   [closed-class.esl `prep_for_sem`/`prep_to_sem`](ontologies/lexicon/closed-class.esl#L845)) against the
   composed-compound NP the shift emits ([lookup.rs:1446](kernel/src/dcg/lookup.rs#L1446)) and against
   the verb's `/NP` object slot. Find the mismatched feature (likely `num`, or the deferred-quant
   raised-cat form the prep slot wasn't built to accept).
3. **Align one slot**: widen the prep's object slot to the shape the verb accepts, *or* have the
   composed-compound shift emit the NP form the prep consumes.

### Acceptance
- `… for <compound>` parses (small-lexicon test); S4 reaches the wide beam, then the page beam after
  Levers 1–2.

### Lands in
A kernel grammar test ([kernel/tests/closed_class_determiners.rs](kernel/tests/closed_class_determiners.rs))
+ either the prep entries ([closed-class.esl](ontologies/lexicon/closed-class.esl#L845), a reseed) or the
shift ([lookup.rs:1446](kernel/src/dcg/lookup.rs#L1446), no reseed).

---

## Sequencing

1. **Lever 1 first** — self-contained, banks the validated S1 win, and gives a *configured* serving path
   to measure the others on. (No reseed; code + wiring only.)
2. **Lever 3** — small and bounded (a kernel test + one slot); a likely no-reseed shift fix.
3. **Lever 2 last and measured** — re-run the cell analysis with Lever 1 active, then a surgical cost
   tweak. May prove small once the sense product is gone.

GH#93 / Lever-B selectional pruning stays **off this corpus's critical path** (valid for general
WordNet `eat`/`think`, recorded in §4a). The MSI/MMR/MSS abbreviation-alias model (#1), OOV import, and
D61 faithfulness check are separate backlog items, independent of these three.

## Diagnostics retained (witnesses; behind `#[ignore]`/`PARSE_DEBUG`, no runtime cost)
`cat_shape` + per-cell shape histograms · `EIGENIUS_DUMP_CELL=i..j` full-category dump ·
`LexicalIndex::debug_form_entries` · and the tests `analyze_chart_cells_first_five`,
`enumerate_function_word_noise`, `verify_sense_lever_at_page_beam`
([db_backed_encoding.rs](crates/eigenius-wordnet/tests/db_backed_encoding.rs)).
