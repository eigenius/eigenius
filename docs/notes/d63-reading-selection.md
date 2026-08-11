# D63 — reading selection (Stage 1 of the parser-pipeline plan)

**Status: design settled 2026-08-11; implementation in progress.**
Parent map: [parser-pipeline-plan.md](parser-pipeline-plan.md). Baseline this stage moves:
46/62 corpus units Ambiguous, 14 Encoded (`experiments/parsing/baseline.json`, 2026-08-02).

## 0. Problem

Every ranking mechanism in the parser operates **pre-parse, on senses per surface word**
(`kernel/src/dcg/sense_ranker.rs` seeds/prunes chart leaves). The assembled reading set — the
`Vec<Item>` a parse returns — is never scored or chosen from. The only existing selector,
`select_pinned` (`crates/eigenius-encoding/src/select.rs`), replays a human-authored skeleton pin
and deliberately fails closed when two readings share it. `resolve_document`
(`kernel/src/dcg/parse/resolve.rs`) classifies `closed.len()>1` as `SentenceOutcome::Ambiguous`
and stops; for discourse threading it harvests entities from an arbitrary `items.first()`.

This stage builds the missing component: an automatic per-sentence reading selector, operating in
document context, recorded and gated.

## 1. The contract

New module `kernel/src/dcg/reading_ranker.rs`, patterned on `sense_ranker.rs`:

```rust
pub struct ReadingCandidate {
    pub skeleton: String,   // skeleton_of(item.sem()) — the sense-erased structure key
    pub gloss: String,      // verbalize(item.sem(), vb) — names concrete senses
    pub sem: String,        // pretty-printed λ-term, for the record + prompt appendix
}

pub struct DocumentContext<'a> {
    pub document: &'a str,        // the full input text
    pub sentence: &'a str,        // the target sentence (marked within the document in the prompt)
    pub prior_selections: &'a [PriorSelection],  // ordinal + gloss of each already-selected reading
}

pub struct ReadingSelection {
    pub chosen: usize,            // index into the candidate slice
    pub rationale: String,
    pub ranked: Vec<usize>,       // full preference order, chosen first
}

pub trait ReadingRanker {
    fn select(&self, ctx: &DocumentContext, candidates: &[ReadingCandidate])
        -> Option<ReadingSelection>;   // None ⇒ abstain ⇒ the sentence stays Ambiguous
}
```

Decisions carried by the contract:

- **Document context is part of the contract, not a hint.** Reading choice (PP attachment,
  coordination scope, sense) is frequently decided by surrounding prose. The ranker sees the whole
  input text with the target sentence marked — preceding *and* following sentences — plus the
  glosses of prior sentences' already-selected readings (sequential consistency: the discourse
  loop has them as it advances).
- **Selection is over FULL READINGS — structure and word senses together.** (Corrected
  2026-08-11: an earlier revision measured accuracy at skeleton granularity, borrowing the pins as
  selection gold. That was a category error — see §5.) Glosses name concrete senses (via the
  promoted verbaliser), so the ranker sees both ambiguity axes; candidates are presented grouped
  by skeleton for legibility, but the choice, and its evaluation, are per reading. Validity —
  never choosing an `invalid`-adjudicated skeleton — is the only structural hard constraint.
- **Abstention is legal.** `None` keeps the sentence `Ambiguous` — fail-open to the current
  behavior, never a forced wrong choice.

## 2. Implementations

| Impl | Role |
|---|---|
| `AnthropicReadingRanker` (`use-llm`) | live selection via `anthropic_client::anthropic_structured`, like the sense ranker |
| `RecordingReadingRanker` / `ReplayReadingRanker` | record to / replay from `selections.json`; the replay key covers **context + candidate set**, so a context or forest change is a counted MISS, never a silent reuse |
| deterministic mock | tests |
| pin-backed selector | wraps the `select_pinned` matching logic; the ground-truth/gate arm |

Unlike `ranks.json` (a pre-dedup instrument — trap §4.3 of the parsing README), `selections.json`
records the **surviving** readings: what the model was asked *is* what was seedable. The pre-dedup
caveat does not transfer.

## 3. Trust story — stated honestly

There is **no kernel veto** on reading selection: every candidate already type-checks, so the
felicity oracle cannot distinguish them. This differs from the two existing LLM seams (a wrong
sense fails to parse; a wrong antecedent fails the re-gate). The controls are:

1. **The recorded decision** — chosen index, rationale, runners-up — lands in the emitted
   `enc:DecisionPoint` (`crates/eigenius-encoding/src/emit.rs`), so every automated choice is
   auditable on the chain.
2. **The offline faithfulness gate** — `selection_accuracy` against the human pins (§5).
3. **The adjudication ledger** — an `invalid`-adjudicated skeleton being *selectable at all* is a
   grammar bug to fix (the ledger's standing rule), not a runtime filter; `invalid_selected == 0`
   is gated to catch it.

## 4. Integration

`resolve_document` gains an optional `&dyn ReadingRanker`. Per sentence, when `closed.len()>1`
and a ranker is present: assemble the `DocumentContext` (the loop already has the full sentence
list and its prior selections) → `select` → `SentenceOutcome::Encoded(chosen)`. Discourse harvest
uses the **chosen** reading — deleting the `items.first()` arbitrariness. No ranker ⇒ exactly the
current behavior (the deterministic no-regression arm; the cap-only sweep is unchanged).

`SentenceEncoding` (`kernel/src/dcg/pipeline.rs`) carries the selection record (source =
pin | ranker | replay, rationale, runner-up skeletons) for emission. Stage 2 widens the selection
pool to resolved-open readings; this stage selects among closed readings only.

## 5. Measurement (corrected 2026-08-11: reading-level, not skeleton-level)

**What the pins are — and are not.** `expected-readings.tsv` is a *grammar-debugging /
faithfulness instrument*: a human verified, per unit, the correct STRUCTURE, and its gate asserts
the forest still *contains* that structure (so parser-rule work can't silently lose correct
readings). It is sense-erased by construction and was never selection gold. In selection it plays
two subordinate roles only: (a) **adjudication evidence** — a chosen reading whose structure
contradicts the verified structure is a wrong reading, no sense judgment needed; (b) the
**structure-correct diagnostic** (reported, not gated).

- **The gated metric is READING-level.** Gold lives in
  `experiments/parsing/reading-adjudications.tsv` — a ledger with one verdict per
  `(sentence, chosen reading's sem)`: `correct` / `wrong` / `uncertain`, with evidence — authored
  by adjudicating recorded draws (a `selections.json` records every candidate's skeleton, gloss,
  AND sem, so any draw is adjudicable after the fact, the same way the pins were authored from
  gloss dumps). The harness scores each chosen reading against the ledger:
  `reading-correct` / `reading-wrong` / `reading-unadjudicated` (an unlisted decision is
  *unadjudicated*, reported — and must be 0 on the tracked replay, else the number is not a
  measurement). `uncertain` rows count as unadjudicated (excluded from the gate's denominator,
  visible in the report).
- **Gates** in `selection-baseline.json` / `scripts/eval-parse-rate.sh` — a SEPARATE baseline
  from `baseline.json`, deliberately: the parse baseline gates the grammar+lexicon (the produced
  forest), the selection baseline gates the ranker (the choice), and they re-baseline on
  different triggers (grammar work vs draw/prompt/ledger changes): `reading-correct ≥
  expected.reading_correct` (a drop = REGRESSION), `reading-unadjudicated == 0` on the tracked
  replay, and `invalid_selected == 0`. `structure-correct` (chosen skeleton == pin, over chosen
  pinned units) is reported as a diagnostic. Measured **with document context** (the corpus page
  in order) — a context-free selection number is not the tracked metric.
- **Recording**: `scripts/measure-parse-rate.sh` writes `selections.json` beside `ranks.json`;
  `--selections` replays it (abstentions are recorded, so a faithful replay has 0 misses). The
  committed baseline value is the drift-free replay, per the standing measurement discipline.
- **Invariant**: parse metrics (grammar-gap, hits, skeletons, readings) are byte-identical with
  and without selection under replay — selection runs after the forest exists.

## 6. Slices

1. **Promote the verbaliser** — DONE (2026-08-11). `kernel/src/dcg/verbalize.rs`; the harness
   imports the kernel version. Public API: `Vb`, `verbalize`, `unit_sense_names`, and the generic
   `resource_label(iri, layer)` (nothing vocabulary-named is public; see §7 preferred-label).
2. **The seam + loop integration** — DONE (2026-08-11). `kernel/src/dcg/reading_ranker.rs`:
   `ReadingRanker`/`ReadingCandidate`/`DocumentContext`/`ReadingSelection`, `PinReadingRanker`,
   `RecordingReadingRanker`/`ReplayReadingRanker` (`selections.json`; the key covers the document
   sha, prior selections, and the full candidate presentation; a replay miss ABSTAINS and is
   counted). `resolve_document` takes `Option<&dyn ReadingRanker>` and returns
   `Vec<SentenceResolution>` (outcome + `SelectionOutcome` audit record: chosen skeleton + gloss,
   rationale, runner-up skeletons, candidate count); candidates are presented grouped by skeleton;
   an out-of-range reply abstains. `InProcessPipeline::with_reading_ranker`;
   `SentenceEncoding.selection`. Tests: seam unit tests + the loop test
   (`a_reading_ranker_collapses_ambiguity_and_an_abstention_leaves_it`).
3. **Harness + gates** — DONE (2026-08-11). The sweep's `Outcome` retains its `Item`s; the
   selection pass runs in document order inside the unit loop via the SAME `Parser::select_reading`
   (now `pub`) the pipeline runs, with the pin-backed arm recording to `selections.json`
   (`EIGENIUS_SELECTIONS_OUT`, set by `measure-parse-rate.sh`; artifacts flush BEFORE
   `assert_replay_faithful`). New summary line `=== SELECTION (…): eligible, chose, abstained,
   curated, correct, invalid-selected ===`; `eval-parse-rate.sh` parses it, gates
   `invalid-selected == 0` (SELECTION-VALIDITY), and gates accuracy once `baseline.json` carries
   `selection_correct`/`selection_curated` (slice 4). VERIFIED on a fresh
   `--umls-all`+alignment reseed (`wordnet-umls-aligned-2026-08-11-consolidated`): the committed
   2026-07-29 ranks replay 62/0, every baseline metric reproduces EXACTLY (gap 0, missing 0,
   encoded 14, readings 761, skeletons 144, hits 60/62) — selection perturbs nothing — and the pin
   arm reads: eligible 46, chose 10, abstained 36 (skeleton ties / unpinned), correct 10/10,
   invalid-selected 0. Also witnessed: on the WRONG snapshot (aligned-d66, 60/60 rank misses) the
   pin arm abstains 51/53 — fail-closed on a divergent forest.
4. **Live ranker** — DONE (2026-08-11). `AnthropicReadingRanker` (`use-llm`): structured reply
   with an explicit `abstain` field; errors and out-of-range replies abstain (never a fabricated
   choice); prompt = the whole document + prior selections + glosses grouped by structure
   (`EIGENIUS_DUMP_SELECT_PROMPT=1` dumps it); live unit test green. Harness arms mirror
   `EIGENIUS_SENSE_RANKS`: `EIGENIUS_SELECTIONS` exists → REPLAY (misses asserted 0), missing →
   LIVE-RECORD, unset → pin-backed; `measure-parse-rate.sh --selections` composes with `--replay`
   so the reference draw ran LIVE selection over the DETERMINISTIC replayed forest.
   **Finding: abstentions must be recorded** — draw 1 could not replay to 0 misses because its 2
   abstentions left no records; `SelectionRecord.abstained` added, draw 2 re-recorded.
   **Reference**: `experiments/parsing/selections/2026-08-11-reference.json` — 46 decisions (44
   selections + 2 abstentions); selection replay 46 hits / 0 misses.
   **Baseline (READING-level, corrected same day — see §5)**: the draw's 44 chosen readings
   adjudicated in `experiments/parsing/reading-adjudications.tsv` (model-adjudicated from glosses
   + sems + WordNet synset lookups, pending human sign-off): **reading-correct 28/44 (64%),
   reading-wrong 16, unadjudicated 0, invalid-selected 0**; structure diagnostic 32/44. The 16
   wrong = 12 structural (pins as evidence: PP attachment, modal scope, governed-complement vs
   adjunct, WRN kind-vs-individual) + **4 sense errors inside the verified structure — invisible
   to any skeleton metric** ('other'→UMLS qualifier junk, 'these libraries'→cDNA Library against
   the ranker's own prior selections, 'MMR deficiency'→Turcot-syndrome alias,
   'relationships'→human_relationship synset). Baseline: `selection-baseline.json` (its own file
   — the ranker's baseline, separate from the parse baseline) with `reading_correct: 28` /
   `reading_adjudicated: 44`; `eval-parse-rate.sh` gates reading-correct,
   `reading-unadjudicated == 0`, and `invalid-selected == 0` (verified engaging:
   `selection 28/44 → 28/44 (holds)`). Parse metrics byte-identical throughout. Stage-1 exit-gate
   criteria are met; slice 5 (emission) completes the stage.
5. **Emission** — DONE (2026-08-11). Vocabulary (`ontologies/encoding/encoding.esl`, chain-loaded,
   not bootstrapped — no reseed): `enc:SelectionAuthority` + individuals `authority_pin` /
   `authority_ranker` / `authority_sole`; `enc:selected_by` (`class_types` + **`allows_only`**
   closing the enumeration — the `reflection:epistemic_status` pattern, so a new authority is a
   deliberate vocabulary edit); `enc:runner_up_skeletons` (`core:value_array`,
   `element_type core:string`). New kernel test `encoding_validates.rs` compiles+validates the
   ontology over its documented chain and pins the closed enumeration (it immediately caught a
   missing `element_type` — the file previously had NO validation coverage outside the demo).
   Emission (`crates/eigenius-encoding`): `ParsedSentence.selection: SentenceSelection` —
   `Pinned` (BYTE-STABLE: no `selected_by`, historical rationale text; regenerating the demo's
   `claims-intact.esl` diffs empty against the committed artifact) / `Ranked` (authority
   individual + ranker rationale verbatim + runner-up skeletons) / `Sole` (authority_sole).
   CLI: `--pins` XOR `--selections` (the computed arm is REPLAY-only here — artifact generation
   stays deterministic; record draws via `measure-parse-rate.sh`); the ranked arm threads prior
   selections in segment order with the recording's 0-based ordinals (part of the replay key), a
   miss ⇒ `SelectError::Abstained`, fail-closed. `select_ranked` wraps the kernel's ONE
   presentation function; `Parser::reading_gloss` made pub for the sole-reading prior gloss.
   Tests: 3 emit-arm unit tests + the ontology test. E2E of the ranked arm over a real store
   lands with Stage 3 (the CLI lacks the harness's Stage-A augmentation, so harness recordings
   don't replay against the un-augmented CLI parser — the known pipeline split, resolved by
   unification).

## 7. Deferred

- Sense-within-skeleton gold labels (adjudicate per-reading within pinned skeletons) — enables
  measuring the sense axis of selection; until then the ranker's within-skeleton choice is
  recorded but ungated.
- **Preferred-label property** (found during slice 1): `kernel/src/dcg/verbalize.rs` carries the
  seeded importers' string conventions — the `wn:`/`umls:` sense-key layouts, the private
  CUI-in-local-name reconstruction (`cui_label`), and two description-format tolerances in the
  generic `resource_label(iri, layer)` (the public label API; nothing UMLS-named is public). No
  crate dependency, graceful fallback, but the kernel is compensating for importers not emitting
  a clean label. Structural fix: importers emit a first-class preferred-label property, read
  generically. Reseed-territory; folded into Stage 2.3 (readable candidate labels), which needs
  the same property.
- Joint document-level selection (choose all sentences' readings simultaneously) — the sequential
  loop with prior-selection context is the v1; revisit if sequential consistency proves weak.
- The multiplicity-reduction levers (pos_prune, mass-shim precision — d63-parse-gap-closure.md §6)
  stay valid independently: fewer candidates make selection easier, but they are not this stage.
