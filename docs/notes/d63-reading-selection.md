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
- **Selection is flat over readings; accuracy is measured at skeleton granularity.** Glosses name
  concrete senses (via the promoted verbaliser), so the ranker sees both ambiguity axes, but gold
  labels exist only per skeleton (the 62 pins). Candidates are presented grouped by skeleton.
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

## 5. Measurement

- **Gold set** (exists, currently consumed only as regression gates):
  `experiments/parsing/expected-readings.tsv` — 62 human-verified correct skeletons;
  `experiments/parsing/adjudications.tsv` — 105 `available` distractors, 15 `invalid` hard
  negatives.
- **New gated metrics** in `baseline.json` / `scripts/eval-parse-rate.sh`:
  `selection_accuracy` = fraction of curated units whose selected skeleton equals the pin, and
  `invalid_selected == 0`. Measured **with document context** (the corpus page in order) — a
  context-free selection number is not the tracked metric.
- **Recording**: `scripts/measure-parse-rate.sh` writes `selections.json` beside `ranks.json`;
  `--replay` replays both. The committed baseline value is the drift-free replay, per the standing
  measurement discipline.
- **Invariant**: parse metrics (grammar-gap, hits, skeletons, readings) are byte-identical with
  and without selection under replay — selection runs after the forest exists.

## 6. Slices

1. **Promote the verbaliser** — move `Vb`, `verbalize`, `unit_sense_names`, `umls_name`,
   `name_atom`, `axiom_local`, `is_false`, `app_spine` from
   `crates/eigenius-wordnet/tests/db_backed_encoding.rs` to `kernel/src/dcg/verbalize.rs`; the
   harness imports the kernel version. (Gate renderer == selector renderer: one function.)
2. **The seam** — `reading_ranker.rs`: types, trait, mock, recording/replay. Unit tests over the
   in-memory demo layer.
3. **Harness + gates** — selection pass in `db_backed_encoding.rs` over the corpus page;
   `selections.json`; `measure-parse-rate.sh`/`eval-parse-rate.sh`/`baseline.json` wiring.
4. **Live ranker** — `AnthropicReadingRanker`; record a reference draw; measure
   `selection_accuracy`; set the gated baseline from its replay.
5. **Emission** — `DecisionPoint` computed-choice arm; ranked path in `select.rs` beside
   `select_pinned`.

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
