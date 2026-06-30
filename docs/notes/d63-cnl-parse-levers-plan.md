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

**The S4 structural fix (future Lever 2 work):** stop the compound/adjective rules from chaining across
tokens that have a **grammatical (closed-class) or verbal** role — a *targeted* guard on the compound
rule (not the reverted blanket closed-class-wins, which wrongly dropped needed open-class senses like
the `be`-verb) — and/or **cost-penalize compound/refinement depth** so the `cat_s` reading outranks the
deep noun-piles within the forest cap. This is the genuine nominal-residual reduction; the dual-POS /
shift-fan-out items below are part of the same explosion.

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
