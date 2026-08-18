# Work stack — unfinished work (top = active)

The single "where are we" pointer. A **LIFO stack** of the active working notes: work the **top** entry;
when its exit-gate is met, **pop** it and the entry below becomes active. When a sub-task splits off from
an entry, **push** its note on top. Keep this file current — it is the map back to the base plan after
any detour.

---

## Stack (top → bottom)

### 0. ▲ ACTIVE — [parser-pipeline-plan.md](parser-pipeline-plan.md) — **Stages 1–3 done; Stage 4 next, behind D69**
The approved four-stage build map (`2026-08-11`): **reading selection → anaphora completion (D64)
→ unified landing (Derived, artifact-first) → FormalizeDocument institution**. It is the successor
spine for the entries below: Stage 1 is the AMBIG→ENCODED exit gate of (2)'s phase 3 — a
*selection* stage (LLM `ReadingRanker` in document context, recorded + gated against the 62-pin
gold set) rather than further multiplicity reduction; Stages 3–4 subsume (3)'s Phase-1-harness and
Phase-2 items. Settled: Derived landing shape; selection inside the discourse loop; document
context for both LLM stages. Design note `d63-reading-selection.md` written; slices 1–3 DONE `2026-08-11` (verbaliser promoted
to the kernel with a generic `resource_label`; `ReadingRanker` seam + record/replay +
`resolve_document` integration; harness selection pass + `selections.json` + SELECTION summary
line + `eval-parse-rate.sh` SELECTION-VALIDITY gate). Verified on a fresh consolidated reseed
(`wordnet-umls-aligned-2026-08-11-consolidated`, the tracked ranks replay 62/0): every baseline
metric EXACT (gap 0, encoded 14, readings 761, skeletons 144, hits 60/62); pin arm chose 10/46,
correct 10/10, invalid-selected 0. Slice 4 DONE `2026-08-11`: live `AnthropicReadingRanker`
(abstain-capable, prompt = document + prior selections + structure-grouped glosses), reference
draw recorded to `experiments/parsing/selections/2026-08-11-reference.json` (46 decisions incl. 2
abstentions — abstentions are recorded, the draw-1 lesson), selection replay 46/0. **Metric
corrected same day to READING-level** (pins = grammar instrument, not selection gold): the draw's
44 chosen readings adjudicated in `reading-adjudications.tsv` → gated baseline **reading-correct
28/44 (64%), invalid-selected 0** in `selection-baseline.json` — its OWN file, separate from the
parse baseline: parse gates the grammar/forest, selection gates the ranker/choice (structure
diagnostic 32/44; the 4-unit gap = sense errors inside the verified structure, invisible to
skeleton metrics). Gate verified holding. Slice 5 DONE `2026-08-11`: `enc:SelectionAuthority`
enumeration (closed via `allows_only`) + `enc:runner_up_skeletons` in encoding.esl (new
`encoding_validates.rs` kernel test — first validation coverage for that file);
`SentenceSelection::{Pinned,Ranked,Sole}` emission arms (pin arm byte-stable — demo
`claims-intact.esl` regenerates identically); CLI `--pins` XOR `--selections` (replay-only
computed arm). **STAGE 1 COMPLETE** (committed). Stage 2 active: the demonstratives-as-holes design note is
WRITTEN — [d64-demonstratives-as-holes.md](d64-demonstratives-as-holes.md) — decision (A):
demonstratives become RESTRICTOR-TYPED holes via a polymorphic `lexicon:anaphor_of` placeholder
freshened at the felicity gate (full kernel veto: "these findings" resolves only to findings);
plain `the` stays ι; dual-entry and post-parse-substitution alternatives rejected; §5 plans the
reseed + migration (re-pin, ledger, fresh selection draw, both baselines re-derived). Slice 2 (kernel mechanism) DONE `2026-08-11`: `freshen_anaphor_of` at the felicity gate,
restrictor-typed holes, and the **hole-type veto in `resolve_open`** — the slice-2 tests exposed
that β-reduction erases the Π-binder annotation, so the restrictor veto had to be enforced
per-antecedent before application (note §2a; subclass antecedents accepted via
`Layer::is_subclass_of`; two occurrences = two independent holes). Four `yonder`-fixture tests
green. Slice 3 (lexicon swap + reseed + migration) DONE `2026-08-11`: 8 entries swapped, reseed →
`wordnet-umls-aligned-2026-08-11-dem`; ranks replayed 62/0; open 2→20, readings 761→226; 19 pins
re-migrated (hits 60/62 HOLDS); selection re-drawn (21/31 reading-correct, invalid 0); both
baselines re-derived. Typed-skeleton instrument LANDED same day (note §5a: `OpenParse::skeleton` prints hole
types; skeletons 139→144 = recovered structure; ledger + pins cover all 144 — no wave). Slice 4
(discourse close-out) DONE `2026-08-11`: §2.2 pooled closed∪resolved-open competition in
`resolve_document`; §2.3 `Candidate` enum (Individual/Kind, readable labels, proposer selects
by index); the derived-kind-predication coercion in the CHECKER (`kind_of(K) : C` iff
`base(K) ⊑ C`, check-mode-only — note §2b) and the resolution search pre-filtered + bounded
(`MAX_REGATE_ATTEMPTS`, note §2c — first run spent 50 min in the unbounded cross-product, now
25 s/page). DB-backed close-out PINNED (`resolve_document_discourse_close_out`): open 20→**15**
(encoded 12, ambiguous 35, gap 0); the 5 closures include «These data sets…» ENCODED to the
semantically-correct harvested kind. Isolated sweep UNTOUCHED — full replay holds every
baseline exactly (readings 226, skeletons 144, hits 60/62, selection 21/31, eval exit 0).
Residual 15 Opens = named deferrals (claims / plural sets / quantifier witnesses / Σ-restrictor
accommodation — note slice-4 record). §2.4 + §2.5 DONE `2026-08-11` (note slice 5): `ProposeCtx`
carries the ranker's `DocumentContext` + hole type; `Proposal { ranked, rationale, confidence }`;
`RecordingProposer`/`ReplayProposer` (memoizing; refusals replay as hits, misses fail closed);
`AnthropicProposer` context prompt; harness `EIGENIUS_PROPOSALS` three-arm; design doc §3/§4
synced to the as-built Π-carrier. Recency pin (12/35/15/0) holds throughout; 171 suites, clippy
both configs. **STAGE 2 EXIT GATE MET** (corpus page resolves through DB-backed
`resolve_document`, fail-closed preserved; residuals are named deferrals). Stage 3 —
[d67-pipeline-unification.md](d67-pipeline-unification.md) — slices 2–4 DONE `2026-08-12` (note
§7 records): `DerivedClaimGrader` + `GradedClaim` reshape (claim_iri / gate_sentence), emit via
the ONE `cluster()` ctor (byte-identity proven old-vs-new via HEAD-worktree probe), ingest lands
Derived with the `IsDerivedAs` witness asserted; `with_storage` doc branches (+
`with_parser_setup` seam; §7-2 OOM confirmed live) with the DB-backed pipeline test; CLI
re-driven over `DocumentPipeline` (--proposals arm; `select_pinned`/`select_ranked` deleted);
`resolve_document` takes the RAW document (joined-sentences sha would have missed every replay
key); `ResolutionOutcome` binding audit → `enc:AnaphorBinding` (closed BindingAuthority enum,
machine-readable antecedents). Close-out pin (12/35/15/0) + all isolated baselines hold EXACT
(eval exit 0); 171 suites, clippy both configs. Slice 5 (claim antecedents) DONE
`2026-08-12` per [d68-claim-kinds.md](d68-claim-kinds.md) §7: kind = a second `is_a` class
(multi-class inhabitation, no new kernel rule); `enc:Claim` + closed kinds in encoding.esl;
curated alignment layer (`claim-kind-alignment.esl`, probe-derived targets, unaligned senses =
sense discrimination); frame table + recorded `KindClassifier` + Assertion default;
`Candidate::Claim`/`ClaimSet` + the DISTRIBUTIVE set arm (per-member veto, And-fold, one set
per parse) + `ClaimLander` seam + same-kind-run assembly. **Measured: with the tracked kind
draw (`experiments/parsing/kinds/2026-08-12-reference.json`, replay 12/0) ALL FIVE
claim-referent units close — open 15→10 (12/40/10/0, pinned beside the deterministic-floor pin
12/35/15/0 which HOLDS)**; isolated baselines untouched; 171 suites, clippy both configs. Kind
verdicts model-adjudicated pending sign-off. The `2026-08-12-d67` snapshot is canonical (page
replay exact; reseed+alignment proven deterministic). Slice 6 (3.4 artifact + 3.5 acceptance)
DONE `2026-08-12` (note §7-6): demo refreshed on d67 (pins RESTORED to the page-verified
`expected-readings.tsv` shapes; ranks re-recorded; `selections-edited.json` recorded through
the CLI's own new RECORD arm; run.sh's EDITED variant selects by that draw — its pinned
skeleton matches 3 sense-variant readings, a tie a skeleton pin cannot break);
`onco-typed.esl` re-derived to the d67 concepts, and the SERVED load caught a real type error
the in-process build path does not run (`exists x0 : a` with an abstract Σ domain →
`DefinitionMalformed Var(TC#2) ≠ Entity`; the activity concept is now fixed in the definition
body); acceptance passes both ways (in-process Holds/Fails with the diagnostic surfaced —
the run.sh:222 gap; served `run.sh` exit 0, intact COMMITTED / edited REJECTED), both demo
artifacts regenerate BYTE-IDENTICALLY; artifact completeness landed (Stage-A glossary
resources + one `DiscourseUnit`+`CutItem` per non-encoded unit, `enc:cut_ambiguous` /
`enc:cut_unresolved` added, token-bounded OOV attribution, CLI `--partial`), proven to LOAD
through the kernel by `artifact_completeness.rs`. **The COMPOSED configuration is measured for
the first time (review catch: every prior number was a no-ranker floor) — discourse loop + reading
ranker = encoded 50 / ambiguous 1 / open 11 / gap 0 over 62 units, replay-verified (selections
39/0, kinds 47/0, ranks 62/0), vs the 12/40/10/0 floor. Its 39 decisions score 23 pinned-correct
/ 6 wrong / 1 abstained / 9 UNSCORABLE — the pins are isolated-sentence skeletons, so gating the
composed pipeline needs pins for discourse-resolved readings (named, not closed). The sweep's
selection accuracy on d67 reproduces the tracked baseline exactly (21/31).** **STAGE 3 EXIT GATE
MET.** **Active step:
Stage 4 — the `FormalizeDocument` institution (parser-pipeline plan §4); its design note is
the first deliverable.** Remaining deferrals live in D68 §5/§5a (collective/group term +
star coercion, persistent plural referents, hole number) and D67 §8 (reflection source-axis
cleanup); kind verdicts + the selections-edited draw + the demo re-pins await human sign-off.

### 1. [d63-parse-gap-closure.md](d63-parse-gap-closure.md) — **Phase 3 of 4: ambiguity**
Four-phase spine (user directive `2026-07-06`, worked in order — stop detouring):
**OOV ✓ → parsing gaps ✓ → ambiguity (HERE) → performance.**

#### STATUS — measured `2026-07-15`, `main`@`29930e4` (post `dcg-cleanup` merge), snapshot `wordnet-umls-aligned-v3-2026-07-15`
| config | units | GAP | MISSING | AMBIG | OPEN | ENCODED |
|---|---|---|---|---|---|---|
| **reranked** (`--features use-llm`) — *canonical* | 62 | **0** | **0** | 58 | 1 | **3** |
| deterministic (cap-only) — *the no-regression gate* | 62 | — | — | — | — | — |

> **Every sentence PARSES — `grammar-gap 0` and `missing-lexeme 0` (reranked). 3 of 62 resolve to a single
> reading.** The gap/OOV problem is **solved**; the ambiguity problem is **not**. `ENCODED 3/62` is the open front.
> *Deterministic (cap-only) row not re-measured since the alignment-v3 + sense-elimination work — re-run
> `scripts/measure-parse-rate.sh --no-llm` to refresh (last cap-only figure, `07-10` pre-alignment, was ENCODED 0).*

`ENCODED` climbed 1 → 3 (`2026-07-10` → `07-12`, now on `main`). The mover was **sense elimination** — the reranker
may now OMIT an impossible sense (the cap no longer backfills from rejects) and 132 closed-class entries carry a real
`core:description` instead of blank prompt lines; that alone took ENCODED 1 → 3–4 (baseline floor set at 3).
Cross-lexicon alignment (12,450 → 38,389 WordNet↔UMLS merges, v1→v3) cut reading *multiplicity* a few % but did
**not** by itself raise ENCODED — standing verdict, confirmed three times: alignment never reaches a single reading,
**the residual is structural** (readings ≈ skeletons × senses; both axes live, skeletons median 6). Treat ±1 ENCODED
as temp-0 reranker drift, not signal; gap/missing are the load-bearing columns. Full record:
`experiments/parsing/baseline.json`.

#### DONE
- **Phase 1 — OOV: CLOSED.** `missing-lexeme 0`, distinct OOV 0 (Stage-A augmentation grounds the page).
- **Phase 2 — parsing gaps: CLOSED.** `grammar-gap 0` (`20d608e`). History of the 12→…→0 descent and the
  per-gap root causes: **§0 + §3 of [d63-parse-gap-closure.md](d63-parse-gap-closure.md)** — not repeated here.
- **Faithfulness — exclusive-focus `alone` (`22e550a`).** Sentence 3 ("Each event alone does not lead to cell
  death") had **0 universals, a lost negation, and a "Department of Energy" subject**. The reranker was
  *already right* (it ranked `DOE` #19/drop, causative `lead` #0/#1) — the faithful reading existed at **no**
  cap, because post-nominal `alone` had no rule, so widen-on-failure kept lowering the cap until the noun-pile
  was the only complete parse. Fix: `alone` as a bare post-nominal `cat_pp` carrying the opaque
  exclusive-focus operator `ontology:sole` ("this event alone" ≡ "only this event"); reuses the existing
  `RefineKind::PpMod` rule with **zero new parser code**, closed-class ⇒ cap-exempt. Now:
  `∀x:(Σy:event. sole(y)). ¬(x causatively-leads-to cell_death)` — 50/50 readings, **0** noun-pile.

#### NEXT — the exit gate: `AMBIG → ENCODED`
Two concrete levers, cheapest first:
1. **Re-test `pos_prune`** (categorical drop of function-word-as-noun readings; `EIGENIUS_POS_PRUNE`, currently
   default-off). It is *the* lever against the `does→DOE`/`doe`/`DO` noun-pile junk that inflates ambiguity.
   It previously made sentence 3 **unparseable — but only because post-nominal `alone` had no rule. That
   blocker is now gone**, so it is newly viable and untested. Gate on the deterministic sweep (`GAP` must stay 0).
2. **Mass-shim precision fixes** (§6 of the parse-gap note): strictly-uncountable-head test +
   acronym↔domain-word collision filter — kill the spurious `mass` readings that inflate *both* reading count
   and parse time.

Levers already applied (hyphenation, build-then-subsume D3, sense cap/reranker) and the ones ruled out for this
corpus (NF §3.3 adjective rule): **§6/§6a of the parse-gap note** and
[d63-parsing-scale-and-pruning.md §4c](d63-parsing-scale-and-pruning.md).

#### DO NOT RE-TRY
- **Per-span pooled sense cap — tried, measured, REVERTED (`b91e100`).** Pooling the cap across a span's
  candidate lemmas *does* make the reranker's drop-verdict bite (a rank-dropped sense hiding in a sub-cap lemma
  bucket — `DOE` in the 2-entry `doe` bucket — otherwise slips the per-lemma cap). But it **regressed
  `grammar-gap 0 → 1`** (unit 52, *"The MSI relationship compared favourably…"*) by over-pruning a multi-lemma
  span, and it is **unnecessary now that `alone` exists** (the faithful reading is reachable at the tight cap,
  so widen-on-failure never fires and the junk is never admitted). Isolated by reverting *only* the seeding
  code — now the `dcg/parse/` module (the `dcg-cleanup` refactor split the old `lookup.rs`; the pooled
  sense-cap logic is in `parse/seed.rs`) — against the same store. Do not re-land without repeating that A/B.
- Kept from the same session: the **UMLS grammatical-surface filter** (17 surfaces incl. `does not`/`alone`/
  `lead`) in `crates/eigenius-umls/src/convert.rs` — that one is a keeper.

#### GOTCHAS (both cost real time — read before measuring)
- **Counting.** `summarize()`'s per-unit listing enumerates **only AMBIG units**; grammar-gaps print in a
  different format, so grepping `[AMBIG` **silently misses every gap**. Count from the
  `=== WRN first page over FULL lexicon: … grammar-gap N …===` summary line (or the `[unit N] … TAG` lines).
- **Snapshot drift.** A bootstrap-ontology edit changes its content hash, so older snapshots **ManifestDrift** —
  and the harness **SKIPs fail-closed while reporting `ok`**: every `db_backed_encoding` test goes green doing
  nothing. Latest drift: the `dcg-cleanup` merge declared `conn_list` on `lexicon:Conn` (`2026-07-15`), retiring
  the 07-12 snapshots. Two `2026-07-16` bootstrap edits invalidated the chain in turn: the
  definite-referential fix (axiom `ontology:the`) and then the quantifier-determiner fix
  (`several`/`many`/`few`/`most`/`both`). **Current resumable snapshot:
  `wordnet-umls-aligned-v3-2026-07-16-quant`** (reseed `--umls-all` + v3-align, 2.7 GB). Always drive the
  measurement through `scripts/measure-parse-rate.sh` (it sets `EIGENIUS_DB_SNAPSHOT` to the newest
  snapshot); the harness fallback `DEFAULT_SNAPSHOT`
  (`crates/eigenius-wordnet/tests/db_backed_encoding.rs:64`) now points at it.

#### Follow-up spun out of the faithfulness work (not started)
**Pre-nominal `only` / `just`.** Same `ontology:sole` operator (already in the ontology), but they attach
**outside the determiner** ("only [this event]") — NP-level focus, a different rule from `alone`'s N-level
refine (an NP-level rule must reach into the generalized quantifier's restrictor). Deliberately deferred rather
than shipping a mis-shaped N-level `only` that would only cover "the only X". Small, self-contained.

### 2. [d63-next-steps.md](d63-next-steps.md) — the D63 pipeline spine (the base)
The overall sequence that (2) is a detour from — now largely folded into (1)'s Stages 3–4.
Remaining once (2) pops, in order:
**address ambiguity** (0 encoded → clean single parses) + long-sentence perf → **grading-phase gaps**
(Citation grade-climb; graded-props run over the full lexicon, persistent doc layer) → **Phase 2**
(orchestrator / served path). The Phase-1 machinery (reshape, pipeline, grader, ingestion, D47 codec) is
done.

---

## On deck (pushed onto the stack when its step becomes active)

- **Reseed OOM — memory profiling follow-up** ([reseed-oom-memory-investigation.md](reseed-oom-memory-investigation.md)).
  **⚠ POSSIBLY STALE — verify before picking up.** A full `scripts/reseed-lexicon-db.sh --umls-all` ran to
  **completion on `2026-07-10`** (exit 0, 2.9 GB snapshot `wordnet-umls-all-alone-2026-07-10`), i.e. the claim
  below that it "blocks any fresh full reseed" no longer reproduces — likely superseded by the `--out-dir`
  chained-load path. Re-confirm the OOM still happens before investing in the profile.
  *Original:* Full WordNet+UMLS reseed OOMs (~20 GiB) deep into the UMLS load; blocks the at-scale
  re-verification of C3-precision (and any fresh full reseed). Static analysis is exhausted (named resident
  terms sum to ~5–7 GiB vs the 20 GiB OOM; the note's §3 lists what is measured-out — text index, RocksDB
  config, in-memory backend, bounded cache — do not re-tread). **Next action: the jemalloc heap profile in §6**
  (feature-gated `tikv-jemallocator` on `eigenius-cli`, bounded native `serve` + ~10 UMLS chunks + `jeprof`
  flame graph) to name the ~15 GiB owner. Diagnostic already in tree:
  `storage/rocksdb/tests/snapshot_memory_probe.rs`.

- **Phases 3 (ambiguity) + 4 (performance)** — one root cause, worked together once phase 2 pops.
  Concrete first lever: the **mass-shim precision fixes** (d63-parse-gap-closure.md §6 — strictly-
  uncountable-head test + acronym↔domain-word collision filter) to kill the spurious `mass` readings that
  inflate BOTH the reading count (median 105/unit, capped at 256) AND parse time (up to 930 s/unit).
  Backstop = [d63-parsing-scale-and-pruning.md](d63-parsing-scale-and-pruning.md) — the CKY
  chart-explosion sub-project (adaptive supertagging + **intermediate-cell** felicity pruning; GH#97) —
  becomes the top entry when phase 4 is active. The reranker (`--features use-llm`) is the phase-3
  AMBIG→ENCODED metric.

## Parked tracks (real, but off this stack)
Separate threads, not blocking the parse→encode pipeline; pull onto the stack only if picked up:
- **GH#104 — NbE readback panic** (`readback.rs:38`): surface `cell` resolves to UMLS **gene** concepts
  `C1413336`/`C1413337` (TUI **T028**), which are then **applied as functions** → `NotAFunction(ResourceVal(…))`.
  **Pre-existing** (48 panics on the pre-`alone` baseline, 32 on current HEAD — recent work reduced, did not
  cause it) and caught per-candidate, so **no unit is lost** and the sweep still completes. But an ill-formed
  term is reaching readback, so the defect is at the **construction site**, not readback; the `.expect()` is
  also the wrong failure mode. Off the critical path.
- **GH#103 — `CompleteJson` intermittently fails** ("No object generated: could not parse the response",
  patent-analysis notebook). Ruled out: the `main` merge (website-only), reseed/schema explosion (schema is
  class-derived, not chain-derived), `max_tokens` truncation (standalone repro used 304 of 2000 tokens).
  Two real findings: (a) the catch block discards `NoObjectGeneratedError`'s `finishReason`/`usage`/raw `text`,
  making every recurrence undiagnosable; (b) `orchestration/deno.json` pins `ai`/`@ai-sdk/anthropic` to
  **`@latest`** and the Dockerfile never copies `deno.lock` — the container has drifted **two majors**
  (`ai` 6.0.158→7.0.19, `@ai-sdk/anthropic` 3.0.69→4.0.11) and re-resolves on every restart, so local ≠ prod.
- [d61-llm-based-encoding-methodology.md](d61-llm-based-encoding-methodology.md) — grounding-discovery +
  typed decision-making layer (the D61 plan).
- Benchmark pilot (D50/D51) — chem+bio; kernel gaps done, infra gaps remain.
- [d63-passive-voice-handling.md](d63-passive-voice-handling.md) — general passive-voice infrastructure:
  object→subject promotion + agent suppression + `rel(theme, ground)` roles (importer `cat_pss` / a grammar
  passive rule). Serves the denominal phrasal half **and** ordinary passive clauses (`were represented by`,
  `is associated with`, … — in the current grammar-gap list). **Trigger:** closing passive clauses on the
  page, or the denominal phrasal half.
- [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md) — the **spec**: the
  `DenominalElement` table + the `⟦X-E⟧ = ⟦E link X⟧` alignment invariant for the denominal-adjective suffix
  class (`-based`/`-like`/`-mediated`/…). The **compound half is DONE** (compound-morphology §3b, shipped
  `2026-07-05`); the **phrasal** half → d63-passive-voice-handling.md. **Trigger:** after the phrasal half
  lands, to gate the `X-E ≡ E link X` equivalence.
- [d63-lexicon-augmentation.md](d63-lexicon-augmentation.md) — the `DocumentPipeline` generalization for
  **lexical gaps**: `AbbrDef → LexicalBinding{surface, long_form?, grounding}`, the pipeline as a
  lexicon-augmentation transducer (`AugmentOptions`/`LexiconProfile`/seed-in-added-out + the feedback cache),
  two-moment grounding with the concept-convergence invariant (`RecQ DNA helicase → C0084304`). **Trigger:**
  generalizing Stage A / closing `recq` via retrieval-grounding; needs the gene-family source
  ([[gene_family_lexicon_gap]]) + a lexicon/ontology index.

## Completed (record, not work)
- [d69-reading-presentation.md](d69-reading-presentation.md) — **COMPLETED `2026-08-17`.** The ranker's
  question was unanswerable as posed: for «MSI cancer models did not have the exonuclease activity of
  WRN» the prompt carried 120 candidates with 120 distinct sems as only **4 distinct strings**, so the
  reading that landed in the demo artifact was chosen BLIND. Root cause (maintainer): strict
  verbalization is ~a left inverse of parsing, so all readings of one sentence converge by
  construction. Slices 2/5 (`Register::{Surface,Expanded}`, concept legend, injectivity guard, demo v2
  regeneration) done 2026-08-13 — the guard immediately found a second lossy site, a comparative's
  dropped standard, worth 50/1/11 → 51/0/11. Slice 3 done 2026-08-17: selection **21/31 (68%) →
  30/40 (75%)**, structure 23/31 → 33/40, 0 unadjudicated, live run and replay agreeing exactly
  (62/0 ranks, 40/0 selections). So blindness was PART of the earlier error, not all of it — 10
  decisions are still wrong with the pool fully visible. Slice 4 (D69-B two-level presentation)
  implemented, measured and **REJECTED**: 24/40 correct, structure 29/40 — six worse, on the same
  forest and ranks. Flat listing remains default; two-level kept behind `EIGENIUS_SELECT_TWO_LEVEL=1`
  since §5's preferred realisation was a two-CALL ranker and this was the cheap proxy. Logged
  truncation kept ON (D62 no-silent-caps). Detail in §7m.
- [d70-named-entities-syntax-vs-denotation.md](../design/d70-named-entities-syntax-vs-denotation.md) —
  **COMPLETED `2026-08-15`, demo fallout closed `2026-08-17`.** One importer flag (`Concept::symbol`)
  decided BOTH proper-name syntax and entity-vs-kind denotation, so the lexicon offered two of four
  combinations and the corpus kept needing the missing one — a bare-standing KIND-referring NP. Fixed
  at source, two causes: new `lexicon:Num::name` (bare + kind-denoting, no mass claim) granted by
  T047/T191; and T033 Finding removed from `COUNT_VETO_TUIS`, its one motivating collision (`gENE`)
  being handled per-atom by drops.json while the veto blocked head-inheritance for 107591 concepts.
  «MMR deficiency» and «Lynch syndrome» now reach C4522088 / C4552100 instead of Turcot / HNPCC;
  expected-hits 62/62 with an empty miss-set; 0 units discarding their sense ranking; readings 637
  (ceiling 700), skeletons 175. Everything the D69 chase compensated for was a concept that could not
  compose bare. FALLOUT, all closed `2026-08-17`: the bootstrap edit invalidated every snapshot
  (ManifestDrift on the `lexicon` layer), which broke both demos and — silently — the D67 §3.5
  acceptance and the ESL round-trip corpus. v2 re-recorded and verified; v1 RETIRED and deleted
  (sense-erased pins are inventory-dependent; README kept as the record); `acceptance.rs` and
  `esl_round_trip.rs` repointed at v2 (paths AND the `demo:formulas:` → `demo:v2:` namespace);
  `kernel/tests/bootstrap_manifest_pinned.rs` now pins the manifest so the next such edit fails in
  `cargo test` rather than days later by hand. RESIDUE (not blocking, none gated): D2 disease
  kind-vs-entity is unanswerable on this corpus; O4 (a concept's own PT outranking another's CE for
  the same string) unimplemented; abbreviations still reach bare standing via the glossary's `mass`
  inheritance rather than `name`; deep-binder round-trip coverage lost with v1's `rule-general.esl`;
  a lexicon-content change still moves no manifest, so that staleness class has no guard.
- **Phase-2 constructions, Step 5/5b/5c — COMPLETED `2026-07-06`** (uncommitted on `13c5bbe` + the
  refactor on top). RC-6 apposition (`appose_group`, bidirectional concept↔semantic-type felicity),
  comma-list connective inheritance (neutral comma finalized by the trailing `and`/`or`), and the
  **coordination refactor** to core-en's list-with-operator shape (`cat_coord` + `coordinate_prop` +
  `complete_coord`, retiring the eager `coordinate_sem` + the Step-5b n-ary workaround). Together −8
  grammar-gaps (20→12). Kernel lib 1611 + `closed_class` 126 green. Detail in d63-parse-gap-closure.md
  §4 Steps 5/5b/5c.
- [d63-compound-morphology.md](d63-compound-morphology.md) — **COMPLETED `2026-07-05`.** Derived-adjective
  OOV closed (Slices 1–2 + §3b denominal-suffix table + `-like` fix); missing-lexeme 6 → 2 over the
  snapshot. Deferred pieces extracted to the parked tracks above (alignment / passive-voice) and the
  gene-family track ([[gene_family_lexicon_gap]] — `recq`).

## Reference / design notes (consulted, not "work")
Not stack items — background for the above: `d63-{document-preprocessing-scope, kind-predication-reshape,
coren-coupled-port-design, pp-attachment-control-scoping, packed-forest-parsing-blueprint,
cnl-*}`, `d62-*`. Pull in when a step needs them.

---

### Maintenance
- Finishing the top's exit-gate → delete/collapse its entry and promote the next. Note the pop here.
- A new sub-task splitting off the active entry → write its note, push it as the new §1, demote the rest.
- This file is the index; the per-note detail lives in the linked notes, not here.
