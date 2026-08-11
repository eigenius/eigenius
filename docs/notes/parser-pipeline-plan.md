# Parser pipeline — the four-stage build map (approved 2026-08-11)

The spine for finishing the document→chain pipeline: **reading selection → anaphora completion →
unified landing → institution**. Continues D63 phase 3 (ambiguity) and extends
[d63-next-steps.md](d63-next-steps.md); the landing side builds on D66
([d66-definitional-lifting-and-witness-normalization.md](../design/d66-definitional-lifting-and-witness-normalization.md)).
Each stage gets its own design note before code; this file is the map, the notes carry the specs.

## Where the code stands (2026-08-11, `parser-pipeline`)

Coverage is solved: 62/62 corpus units parse (grammar-gap 0, missing-lexeme 0), all 144 skeletons
adjudicated (`invalid` 0), 60/62 expected-reading pins hit (`experiments/parsing/baseline.json`,
2026-08-02). Residual: **46/62 Ambiguous, 2 Open, 14 Encoded**. The remaining stages exist in
fragments split across two disjoint pipelines:

- Kernel `DocumentPipeline`/`InProcessPipeline` (`kernel/src/dcg/pipeline.rs`): glossary + parse +
  anaphora; no reading selection — `resolve_document` (`kernel/src/dcg/parse/resolve.rs`)
  dead-ends `closed.len()>1` as `SentenceOutcome::Ambiguous`, resolves anaphora only when a
  sentence has *zero* closed readings (and only `open.first()`), and harvests discourse entities
  from an arbitrary `items.first()` for ambiguous sentences. In-memory doc layer only; test-only.
- Encoding CLI (`crates/eigenius-encoding/src/pipeline.rs`, `prose-to-esl`): selection via human
  pins + recorded ranks (`select_pinned`) + landing (`emit.rs::emit_document`); no glossary, no
  anaphora; fails closed on open parses. This is what the prose-to-formulas demo runs.

Settled decisions (2026-08-10/11):

- Parsed sentences land **Derived** (`enc:EncodedClaim` + `reflection:ProgramTrace` →
  `IsDerivedAs`); the Declared 3-resource cluster is reserved for curator-pinned rules.
- Selection lives **inside the discourse loop**: candidates for sentence N+1 depend on which
  reading of N was chosen, and an anaphoric reading must compete with closed ones.
- Both LLM stages (reading selection, anaphora proposal) operate **in document context** — the
  surrounding input text, not the isolated sentence.
- The full integration is an **institution** (Stage 4), not a CLI subcommand.

---

## Stage 1 — Reading selection

Collapse `Ambiguous(Vec<Item>)` → one chosen `Item` per sentence, automatically, with the choice
recorded and gated against the human gold set. Design note: `d63-reading-selection.md`.

1. **Promote the verbaliser to the kernel** — move `Vb`, `verbalize`, `unit_sense_names`,
   `umls_name`, `name_atom`, `axiom_local`, `is_false`, `app_spine` from
   `crates/eigenius-wordnet/tests/db_backed_encoding.rs` into `kernel/src/dcg/verbalize.rs`. All
   dependencies are kernel types (`Parser::debug_form_entries`, `Layer`, `Exp`). Same argument as
   the 2026-07-25 skeleton move: the gate's renderer and the selector's renderer must be ONE
   function.
2. **`ReadingRanker` seam** (`kernel/src/dcg/reading_ranker.rs`, patterned on `sense_ranker.rs`):
   `select(doc: &DocumentContext, &[ReadingCandidate]) -> ReadingSelection`. `DocumentContext` =
   the full surrounding input text with the target sentence marked, plus the glosses of prior
   sentences' already-selected readings. `ReadingCandidate { skeleton, gloss, sem }`, grouped by
   skeleton; returns chosen index + rationale + ranked runners-up. Impls: `AnthropicReadingRanker`
   (`use-llm`), `RecordingReadingRanker`/`ReplayReadingRanker` (`selections.json`, replay key
   covers the context — a context change is a counted MISS), deterministic mock, pin-backed
   selector wrapping `select_pinned`. **Trust story:** no kernel veto exists here (every candidate
   type-checks); controls are the recorded decision + rationale, the offline faithfulness gate,
   and the adjudication ledger (a selectable `invalid` skeleton is a grammar bug, not a runtime
   filter).
3. **Integrate into `resolve_document`**: optional `&dyn ReadingRanker`; `closed.len()>1` + ranker
   → select in context → `Encoded(chosen)`; harvest from the chosen reading. No ranker ⇒ current
   `Ambiguous` behavior (the deterministic no-regression arm). `SentenceEncoding` carries the
   selection record.
4. **Eval harness + gates**: gold set already exists — `expected-readings.tsv` (62 pins) +
   `adjudications.tsv` (105 `available` distractors, 15 `invalid` hard negatives). New gated
   metrics in `baseline.json`/`eval-parse-rate.sh`: `selection_accuracy` (selected skeleton ==
   pin; measured **with document context**, at skeleton granularity) and `invalid_selected == 0`.
   Parse metrics must be identical under replay — selection runs after the forest exists.
5. **Emission**: `enc:DecisionPoint` (`crates/eigenius-encoding/src/emit.rs`) gains the
   computed-choice arm (source = pin | ranker | replay, rationale, runners-up).

Exit gate: selection accuracy measured and gated on the 62-unit gold set; parse metrics unchanged
under replay.

## Stage 2 — Anaphora completion (D64)

The corpus page's actual anaphora resolves — demonstratives above all — inside the
selection-integrated loop. The LLM's role is already the design (D64): the proposer selects the
referent among assembled candidates; the kernel re-gate (β-apply into the Π-carrier, re-check,
fail closed) is the veto. Design note: demonstratives-as-holes (bootstrap edit ⇒ reseed, batched).

1. **Demonstratives → the resolver**: today `this/that/these/those` denote `ontology:the(N)` (ι,
   `ontologies/lexicon/closed-class.esl`) — closed, so ~19 units ("These findings", "This state")
   never reach D64. The note argues: (a) hole typed by the restrictor N (the Π-carrier already
   types holes; the re-gate then enforces "these findings" → a finding), (b) dual entries
   competing under selection, (c) post-parse substitution at ι sites. Leaning: the hole
   discipline. Plain definite `the` stays ι.
2. **Closed-vs-open competition**: per sentence, pool = closed readings ∪ open readings whose
   holes successfully resolve (each open parse tried via `resolve_with`); the Stage-1 ranker
   selects over the pool. Unresolvable holes drop a reading — the kernel veto as selection filter.
3. **Candidate quality**: readable surfaces from layer labels (reuse the promoted naming helpers);
   `Candidate` becomes an enum — named individual, kind, landed claim — so "These findings"
   resolves to prior sentences' committed claims (needs Stage 3's incremental landing for the
   claim IRIs).
4. **Proposer upgrades**: `ProposeCtx` gains the `DocumentContext` + hole type/number features;
   `AnthropicProposer` reply gains confidence + rationale (recorded, replay-keyed); optional type
   pre-filter of candidates.
5. **Tests + docs**: DB-backed `resolve_document` over the corpus page; `use-llm` multi-sentence
   live test; update D64 §3 to the Π-abstraction carrier (in code since 2026-07-23).

Exit gate: corpus page resolves through DB-backed `resolve_document`; fail-closed preserved; gate
re-run green on the post-reseed snapshot.

## Stage 3 — Pipeline unification + Derived landing

The unified pipeline generates the parsed document as a **resource set (ESL / Eigon-JSON)** that
commits by loading through the kernel. Design note: pipeline-unification + artifact shape.

1. **Persistent doc layer** (`with_storage`, today only a comment in `kernel/src/dcg/pipeline.rs`):
   the doc-glossary layer commits onto a `doc-<id>` branch of the persisted store (branch plumbing
   exists), replacing the in-memory overlay that OOMs over a DB-backed base.
2. **Fold selection + emission into the trait pipeline**: `InProcessPipeline` gains the ranker +
   proposers; the encoding CLI becomes a thin driver over `DocumentPipeline` (its
   pins+recordings mode = the deterministic replay arm).
3. **`DerivedClaimGrader`** (`crates/eigenius-reasoning/src/grade.rs`, implements `ClaimGrader`):
   the one source of the Derived cluster. emit.rs's claim resources move into the grader;
   document structure (`DiscourseUnit`/`ScopedUnit`) + `DecisionPoint` stay emission-side;
   `ingest.rs` switches parsed sentences to it. Witness side needs no change
   (`hash_stored_proposition` already decodes before hashing; `ProgramTrace` → Derived).
4. **The landing artifact**: the whole document as a generated resource set — glossary resources,
   discourse units, Derived claim clusters, `DecisionPoint`s, resolved anaphora bindings —
   generalizing `prose-to-esl`'s output. Committing = loading the artifact through the kernel
   (validator, Rule 22, the D39 gate), exactly the demo's pattern; generation and commitment stay
   decoupled, the artifact inspectable. The driver reports per-sentence outcome + claim IRI +
   verdict; the `qc_validate_justification` `Fails` diagnostic gets surfaced through the load path.
5. **Acceptance**: reproduce the demo's core programmatically — both paragraph variants land, the
   hand-authored rule + inference files load on top, intact justifies twice / edited `Fails`.
   Def/rule/certificate *generation* stays out of scope (D66 §7).

Exit gate: full workspace tests + clippy clean + the acceptance run over the generated artifact.

## Stage 4 — The pipeline as an institution

The full integration is the D62 `FormalizeDocument` institution (D64 already places anaphora as
its S3 step), realized via the D14 mechanism: prose enters as an `ImportFormat`, the landed
resource set is the export, and the LLM steps (sense rank, reading selection, anaphora proposal)
become orchestrator-backed proposer impls behind the existing seams (the Phase-2 axis of
d63-next-steps.md). First deliverable is its design note: institution signature, doc-branch home,
verdict surfacing, orchestrator hosting of the proposers. The Stage-3 artifact and drivers are
what the institution wraps — nothing in Stages 1–3 is throwaway.

---

## Order and dependencies

Stage 1 first (it is the active work-stack phase — ambiguity). Stage 2's competition rework (2.2)
depends on the Stage-1 ranker; 2.1's reseed is batched with other pending bootstrap edits. Stage
3's grader work (3.3) is independent of anaphora and can run in parallel with Stage 2. Stage 4
starts only once the Stage-3 artifact path is proven.
