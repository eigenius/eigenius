# D67 — Stage 3: pipeline unification + Derived landing (parser-pipeline plan §3.1–3.5)

**Status: design note for review — precedes any code.** Parent map:
[parser-pipeline-plan.md](parser-pipeline-plan.md) Stage 3. Settled inputs: parsed sentences land
**Derived** (user decision 2026-08-10; the Declared cluster is reserved for curator-pinned rules);
selection lives inside the discourse loop (Stage 1); anaphora resolves inside it (Stage 2, exit
gate met 2026-08-11 — open 20→15 on the page, residuals = claims / plural sets / quantifier
witnesses / Σ-restrictor accommodation).

## 0. Problem: two disjoint pipelines, two claim shapes

- **Kernel `DocumentPipeline` / `InProcessPipeline`** (`kernel/src/dcg/pipeline.rs`): glossary +
  parse + selection + anaphora — the full Stage-1/2 loop — but the doc layer is an **in-memory
  overlay** (OOMs over a DB-backed base, the §7-2 caveat; `with_storage` exists only as a
  comment), and nothing lands: `SentenceOutcome::Encoded` is where it stops.
- **Encoding CLI** (`crates/eigenius-encoding/src/pipeline.rs`, `prose-to-esl`): parses its own
  way (per-sentence forests, pins XOR replayed selections), **fails closed on open parses** (no
  anaphora), and emits the five-resource record (`emit.rs`) — the demo's `claims-intact.esl`.
- **Ingestion** (`crates/eigenius-reasoning/src/ingest.rs`): composes the kernel pipeline with a
  `ClaimGrader`, but the only grader is `DeclaredClaimGrader` — parsed sentences land as the
  3-resource **Declared** cluster (DeclaredResource + DeclarationTrace + ReasoningSentence),
  which contradicts the settled Derived decision.
- **Two claim-cluster constructions exist**: `emit.rs` hand-builds the Derived pair
  (`enc:EncodedClaim` + `reflection:ProgramTrace`) inline; `grade.rs` builds the Declared triple.
  The Derived shape has no grader, so ingestion cannot produce it and emission cannot be reused.

Stage 3 unifies these: **one grader for the Derived shape (§1), one persistent doc-layer path
(§2), one pipeline the CLI drives (§3), landing that happens inside the discourse loop so claims
become anaphora antecedents (§4), and the whole document as one generated, kernel-loadable
artifact (§5).**

## 1. `DerivedClaimGrader` — one source for the landed shape (plan 3.3)

New grader in `crates/eigenius-reasoning/src/grade.rs`, implementing the existing `ClaimGrader`
trait. The Derived cluster is **two** resources:

1. the **`enc:EncodedClaim`** — `reflection:canonical_proposition = P` (D47-encoded once, reused
   verbatim — the gh #75 same-bytes invariant);
2. its **`reflection:ProgramTrace`** — `reflection:resource → claim`,
   `reflection:source = <program provenance>`, `reflection:timestamp` — which mints
   `IsDerivedAs(claim_iri, P)` into the witness index at commit. Downstream certificates cite
   `derived(claim_iri, P, _)`; witness admission needs no change (`hash_stored_proposition`
   decodes before hashing; `trace_category` already maps ProgramTrace → Derived).

Two API consequences, both breaking (pre-production, fine):

- **`GradedClaim` reshape.** Its `sentence_iri` field conflates two roles that only coincide for
  the Declared triple: the IRI downstream certificates *cite* (Declared: the declaring resource;
  Derived: the claim) and the resource the D39 gate *validates at commit* (Declared: the
  ReasoningSentence; Derived: none — a ProgramTrace mints its witness without a certificate to
  check). Split them:

  ```rust
  pub struct GradedClaim {
      pub resources: Vec<Resource>,
      /// The IRI downstream certificates cite (`declared(iri,…)` / `derived(iri,…)`).
      pub witness_iri: Iri,
      /// The ReasoningSentence the D39 gate validates, when the cluster carries one
      /// (Declared and inference clusters). `None` for Derived — nothing to gate.
      pub gate_sentence: Option<Iri>,
      pub grade: Grade,
  }
  ```

  `ingest.rs`'s validation loop keys on `gate_sentence`; a Derived claim's verdict stays `None`
  (its trust story is the trace, not a certificate).

- **`ClaimSource` gains provenance.** The ProgramTrace's `reflection:source` must say *which
  program derived the claim from which bytes* — `emit.rs` today writes
  `"…DCG parse (D63) of {path} chars {a}..{b} (source sha256 {sha})"`. Add
  `pub provenance: &'a str` to `ClaimSource`; `DerivedClaimGrader` writes it as the trace's
  `source`, `DeclaredClaimGrader` ignores it. (`declared_by` stays — it is the Declared
  cluster's REQUIRED field and means something else: *who* asserts, not *what computed*.)

**Emission split.** The claim cluster moves into the grader; document structure stays
emission-side. `emit_document` builds each sentence's cluster via `DerivedClaimGrader` and then
sets the two document-structural fields on the returned claim resource (`enc:from_unit → scoped`,
`core:description`) before pushing — grader = epistemics, emitter = document structure.
**Byte-stability gate**: the pin-arm demo artifact (`claims-intact.esl`) must regenerate
byte-identically through the refactor (same field set, same strings); this is asserted by the
existing demo regeneration, re-run in the slice.

**Ingestion switch.** `ingest.rs` grades parsed sentences with `DerivedClaimGrader`
(`Warrant`/`Grade::Derived`); `DeclaredClaimGrader` remains for curator-pinned rules (the demo's
hand-authored rule file). `Warrant` gains the variant; the grade projection stays structural.

## 2. Persistent doc layer — `with_storage` (plan 3.1)

Build the constructor sketched at `kernel/src/dcg/pipeline.rs:71-74`: over a DB-backed base the
doc-glossary (and the claims layer on top of it) commit onto a **branch** of the persisted store
instead of the in-memory overlay that OOMs.

- **Shape**: `InProcessPipeline::with_storage(store, doc_id)` (builder-style, like
  `with_reading_ranker`). Stage A builds the glossary layer **on the store's storage** and
  commits it to branch `doc-<doc_id>` off the current lexicon head; parsing runs over the
  committed branch head. The invariant that decides the mechanics is the index lifecycle one:
  derived indexes populate in `store_layer` (post-validation) — **build the layer on the storage
  it is persisted to**, never build-in-memory-then-copy. The branch plumbing exists
  (`CreateBranch` → `Load(branch)` → parse — d63-next-steps.md Phase 2, noted working).
- **Lifecycle**: a rerun of the same `doc_id` **drops and recreates** the branch
  (pre-production posture; the artifact (§5) is the reproducible record, the branch is working
  state). The interactive chain (`main`) is never advanced by the pipeline — landing onto it is
  an explicit downstream `eigenius load` of the artifact.
- The in-memory path stays the default for tests and small bases; `with_storage` is opt-in.

## 3. One pipeline; the CLI becomes a thin driver (plan 3.2)

- `InProcessPipeline` already carries the anaphora proposer and the optional `ReadingRanker`
  (Stages 1–2). What is missing is the **landing stage**: after resolve+select, grade each
  `Encoded` sentence via the grader (§1) and emit the document record (§5). `ingest.rs` keeps
  the graded-claims contract; the emission path moves behind the same pipeline.
- **The encoding CLI** (`prose-to-esl` / `prose-to-eigon`) re-drives over `DocumentPipeline`
  instead of its private parse loop. Its deterministic replay arms map 1:1 onto the pipeline's
  existing seams: `--ranks` → replay sense ranker, `--selections` → `ReplayReadingRanker`,
  `--pins` → `PinReadingRanker` (already the pipeline's gate arm), and (new) `--proposals` →
  `ReplayProposer`. Fail-closed behavior changes deliberately: an open parse no longer aborts
  the run — it resolves through the discourse loop or lands as an honest `Open` unit in the
  report (the artifact records the outcome; a partial *encoding* is still never emitted as if
  complete).
- **Resolution audit (kernel piece).** The chosen anaphora bindings are currently lost —
  `resolve_open` returns a bare `Item`. For the artifact (§5) and the claim record,
  `resolve_with` returns the bindings alongside: `SentenceResolution` gains
  `resolution: Option<ResolutionOutcome>` with
  `ResolutionOutcome { bindings: Vec<(String /*hole var*/, Candidate)> }` — the audit sibling
  of `SelectionOutcome`. (The §2.4 proposal record stores what was *asked*; this stores what the
  kernel *accepted*.)

## 4. Incremental landing — claims become antecedents (closes §2.3's claim gap)

**GATED on §8** (review 2026-08-11): the claim-antecedent slice does not build on the current
`enc:EncodedClaim` shape — the corpus shows it cannot type discourse reference correctly (a flat
claim class under a *finding* alignment would let «these findings» resolve to a hypothesis).
The mechanics that survive the revision:

- **Landing inside the loop.** The claim cluster for sentence *i* is built (grader, §1) as soon
  as its outcome is `Encoded`; its IRI + gloss join the discourse candidate set for sentence
  *i+1*. `Candidate::Claim` **carries the built `Resource` directly** (the `Candidate::Kind`
  pattern — the term travels with the candidate): `antecedent_exp` embeds
  `Exp::EigonResource(..)` with no layer lookup, the checker's inhabitation rule reads `is_a`
  off the embedded resource, and only the subsumption walk consults the chain — so no
  per-sentence re-indexing; commit order alone preserves Rule-22 reference integrity when the
  referring sentence lands. The claim-kind classes (§8) must be on the parse chain.
- **Plural/group reference is built properly, not approximated** (decision 2026-08-11): a
  plural demonstrative's referent is a SET of claims (units 19/20 refer to the claims of units
  13–18), and binding it to one claim would be a wrong closed parse regardless of audit trail.
  Slice 5 includes a set/group antecedent term — the natural candidate is the coordination
  group representation the grammar already builds for "A, B and C" NPs, typed distributively
  (each member inhabits the restrictor class); the exact term shape is pinned in §8's revision
  note against the coordination sems.
- **Measured** via the discourse close-out: the 4 claim-referent units are the target; the
  pinned numbers re-ratchet with provenance.

## 5. The artifact: the document as one generated resource set (plan 3.4)

The pipeline's first-class output is a **generated ESL / Eigon-JSON resource set** for the whole
document — generation and commitment stay decoupled; **committing = loading the artifact through
the kernel** (validator, Rule 22, the D39 gate), exactly as the demo does. Contents, per
sentence group under one `--ns` root:

- Stage-A glossary resources (the abbreviation/named-entity bindings that grounded the parse —
  `abbreviation_resources`/`glossary_resources`, today dropped after parsing);
- `enc:DiscourseUnit` + `enc:ScopedUnit` (unchanged shape);
- the **Derived claim cluster** (§1) for closed sentences;
- `enc:DecisionPoint` (selection authority — unchanged shape, pin arm byte-stable);
- **new: `enc:AnaphorBinding`** — one per resolved hole: the scoped unit, the hole var, the
  antecedent recorded machine-readably (a `ResourceRef` for individuals/claims; the
  D47/`type_expr` encoding for kind terms, which have no IRI — plus the display surface), the
  proposing authority (recency / proposer / replay), and the proposer's rationale/confidence
  when present (§2.4 record). Closed enumeration + `class_types`/`allows_only` per the
  encoding.esl conventions; `encoding_validates.rs` extends.
- Sentences that did not close land as `DiscourseUnit`s with their outcome recorded — the
  artifact states honestly what did not encode; it never silently drops a unit.

## 6. Acceptance (plan 3.5) + verification

- Programmatic reproduction of the demo core: both paragraph variants → pipeline-land on
  branches → the hand-authored rule + inference files load on top → intact justifies twice /
  edited `Fails`; `qc_validate_justification`'s `Fails` diagnostic surfaces through the load
  path (the run.sh:222 gap). `demo/prose-to-formulas/run.sh` still passes end to end;
  `claims-intact.esl` regenerates byte-identically under the pin arm.
- Full workspace tests + clippy clean; the parse/selection baselines hold EXACTLY under replay
  (landing runs after the forest exists); the discourse close-out re-ratchets only at §4's
  claim-candidate slice, with provenance.

## 7. Slices (each independently verifiable)

1. **This note** — review gate (reviewed 2026-08-11; §8 added from review).
2. **3.3 grader** — DONE (2026-08-12). `GradedClaim` reshaped: `claim_iri` (the resource
   carrying the proposition — the declaring resource / the `EncodedClaim` / the sentence, per
   cluster; renamed from the note's `witness_iri` at implementation, since "what certificates
   cite" is per-relationship for the Declared cluster while "the proposition's carrier" is
   uniform) + `gate_sentence: Option<Iri>` (`None` for Derived — ingest's validation loop keys
   on it; a Derived claim's verdict stays `None`). `ClaimSource` gains `provenance` (the
   trace's `reflection:source` line); `Warrant::Derived` projects `Grade::Derived`.
   `DerivedClaimGrader` with the shared **`cluster()`** constructor — ONE source of the
   claim+trace shape, TWO naming policies (trait path: `{stem}:claim`/`{stem}:trace`; the
   emitter keeps its historical `{ns}:claim_{n}`/`{ns}:trace_{n}`). `emit_document` builds via
   `cluster()` + adds only `from_unit`/`description`; **byte-identity proven old-vs-new** (a
   HEAD worktree ran the fixture through the pre-refactor emitter; diff empty across all three
   selection arms). `ingest.rs` lands parsed sentences Derived; its test asserts the
   `IsDerivedAs(claim, P)` witness is ADMITTED on the committed chain
   (`layer_admits_witness`). *Caveat discovered:* the demo's artifact-level regeneration gate is
   UNAVAILABLE — the `wordnet-umls-aligned-d66` snapshot ManifestDrifts against the Stage-2
   bootstrap (the dem migration edited `closed-class.esl`), and the dem snapshot's ALIGNMENT
   differs (the demo's rank keys miss; its pinned forest is not reproduced). The demo needs a
   fresh aligned reseed on the current bootstrap + re-recorded ranks + re-verified pins —
   follow-up, not this slice's regression.
3. **3.1 persistent doc layer** — DONE (2026-08-12). `InProcessPipeline::with_storage(backend,
   doc_id)`: the doc layer is built ON the store's storage (`LayerStorage::with_persistent` —
   the index-lifecycle invariant) and committed via `BackendPersister` to `doc-<doc_id>`,
   pre-pointed at the base head so the CAS creates-or-replaces deterministically
   (drop-and-recreate lifecycle; `main` never advanced). `DocumentPipeline::encode` became
   fallible (`PipelineError::Persist`). A gap surfaced and closed: the pipeline had NO
   parser-configuration seam — `with_parser_setup` hook added (caps + rank replay are
   load-bearing over the full lexicon). Measured (`pipeline_with_storage_commits_doc_branch`,
   dem snapshot): page through the pipeline in 39 s, branch points at the committed layer,
   rerun re-points it; outcome tally (4/34/19/5) differs from the sweep as documented — the
   pipeline's `DocumentOnly` Stage A is a different overlay (no named-entity / source-abbrev
   groundings), so the test gates structure, not parse numbers.
4. **3.2 unification** — DONE (2026-08-12).
   - `resolve_document` takes the **raw document text** as a parameter (found: it synthesized
     `sentences.join(" ")`, whose sha differs from the raw page every recording keys on — every
     selection/proposal replay through the pipeline would have silently missed).
   - The binding audit: `ResolutionOutcome { bindings: Vec<ResolvedBinding> }` (hole, accepted
     candidate, proposer rationale/confidence) returned by `resolve_with`, threaded through the
     pool to `SentenceResolution.resolution` / `SentenceEncoding.resolution` — recorded only
     for the ENCODED reading (Ambiguous is fail-open terminal).
   - The CLI is a thin driver over `DocumentPipeline`: `--pins` → `PinReadingRanker`,
     `--selections` → `ReplayReadingRanker`, `--ranks` → the parser hook
     (`build_sense_ranker`), new `--proposals` → `ReplayProposer` (replay-only; record with the
     close-out harness's `EIGENIUS_PROPOSALS` arm). Fail-closed diagnostics re-derived from
     pooled outcomes (pin-miss/tie, replay abstention, open holes, gaps);
     `select_pinned`/`select_ranked` DELETED — `select_reading` + the rankers are the one
     selection path; `select.rs` keeps only the pin-file format. The CLI goes through
     `with_storage` on the snapshot working copy — the first live confirmation of §7-2: the
     in-memory doc layer over the DB-backed base was SIGKILLed (build-time index population
     walks the full chain).
   - `enc:AnaphorBinding` vocabulary (closed `BindingAuthority` enumeration
     recency/proposer/replay via `allows_only`; `antecedent_resource` for individuals/claims,
     `antecedent_term` = the D47 `eigentt:TypeExpr` encoding for kinds; `enc:confidence`
     domain extended) + emission (one binding resource per resolved hole, after the
     DecisionPoint; empty for closed readings — pin-arm bytes unchanged) +
     `encoding_validates` pins the enumeration.
   - Verified: 171 workspace suites; clippy `-D warnings` both configs; the discourse
     close-out pin HOLDS (12/35/15/0) under all of slices 2–4; the isolated-sweep replay holds
     every baseline (run recorded in `experiments/parsing/results/`).
5. **§4 claim antecedents** — GATED on the §8 revision note: claim kinds + set antecedents +
   per-kind alignment, then incremental landing into the candidate set and the close-out
   re-measure (re-ratchet with provenance).
6. **3.4/3.5 artifact + acceptance** — DONE (2026-08-12), on the `2026-08-12-d67` snapshot.
   - **Demo refresh** (the slice-2 caveat, closed). `pins.tsv` re-verified: the full-scope d67
     alignment RESTORES the page-verified shapes, so the three pins are `expected-readings.tsv`
     (2026-07-22) verbatim and the 2026-08-03 narrow-scope re-pins are retired. `ranks.json` /
     `ranks-edited.json` re-recorded live; `selections-edited.json` recorded through the CLI's
     own RECORD arm (added this slice: selection keys hash the PRESENTED pool, and the
     measurement harness's Stage A is not the CLI's — a draw recorded there cannot answer this
     driver's questions). `run.sh --reparse`'s EDITED variant now selects by that draw, not by
     pin: on d67 the negated sentence's pinned skeleton matches **3** readings differing only
     in sense — a tie a sense-erased pin cannot break (fail-closed, correctly).
   - **`onco-typed.esl` re-derived, and the served path caught a real type error.** The d67
     alignment gives «exonuclease activity» / «helicase activity» dedicated UMLS concepts
     (C1148824 / C1149627) instead of compound readings, so the Σ domain is the activity
     concept itself. First rewrite abstracted it as a parameter (`HasActivity(m, g, a : Set)`,
     `exists x0 : a => …`): it compiles in-process but `eig load` refuses it —
     `DefinitionMalformed: Var("TC#2") ≠ EigonClass(lexicon:Entity)`. Correct: `fst(the(Σ x0:a. …))`
     has type `a`, and an abstract `a : Set` has no subsumption path to the verb axiom's
     `Entity` slot, which a concrete class has. The definitions FIX the activity concept in the
     body (`HasActivity(m, g)` / `RequiresActivity(m, g)`); the in-process build path does not
     run this validation, so only the served run found it.
   - **3.5 acceptance, two ways.** In-process (`tests/acceptance.rs`, `--ignored`): both variants'
     claim layers land on branches of the working copy, the vocabulary + rule + inference load
     on top, `do_validate_justification` gives **Holds** for intact and **Fails** for edited —
     with the diagnostic surfaced (`no admitted IsDerivedAs witness for … claim_1`), which is
     the run.sh:222 gap closed on the in-process side. Served: `run.sh` end-to-end exit 0 —
     intact ✓ COMMITTED, edited ✓ REJECTED at the AutoOnLoad gate. Both committed artifacts
     (`claims-intact.esl` / `claims-edited.esl`) regenerate **byte-identically** under the new
     emitter.
   - **3.4 artifact completeness.** `emit_document` gained two inputs: the Stage-A **glossary**
     resources (`LexiconAugmentation::resources()` — the entries that grounded the parse; the
     artifact was not self-contained without them, a claim's proposition can reference a
     doc-glossary-only concept) and the **cuts** — one `enc:DiscourseUnit` + one `enc:CutItem`
     per non-encoded unit. Two `CutKind` individuals added (`enc:cut_ambiguous`,
     `enc:cut_unresolved`) beside the existing grammar/vocabulary/out-of-scope; a no-parse is
     classified vocabulary-vs-grammar by whether a residual Stage-A OOV surface occurs in the
     sentence as a whole token (substring matching credited «then» to «strengthen»).
     CLI `--partial` records instead of aborting, and makes the selection authority optional
     (neither `--pins` nor `--selections` ⇒ sole survivors only); a pin CONTRADICTION stays
     fatal under `--partial` — that is pin drift, not a coverage gap. Measured over the corpus
     page (cap-only, no ranker, no proposals — the weakest configuration on purpose): 30 units,
     every one recorded — 12 no-parse-vocabulary, 6 no-parse-grammar, 4 ambiguous, 8
     unresolved-referent, 0 encoded — plus 4 glossary resources.
     `tests/artifact_completeness.rs` (`--ignored`, DB-backed) proves the shape LOADS: glossary
     + all four cut kinds → ESL printer → `compile_against_layer` → validated layer build, each
     resource resolvable on the result.
   - **Harness trap fixed while re-verifying the close-out pin.** `resolve_document_discourse_close_out`
     defaulted to `WRN_PAGE` (the RAW page) while its assertions are calibrated on the CNL
     rewrite: run without `EIGENIUS_WRN_PAGE` it reports 18 grammar gaps over 30 units and trips
     `pooling must not create grammar gaps` — a red run that says nothing about the change under
     test. `WRN_PAGE_CNL` added and made this test's default (env still overrides; the
     structural tests keep the raw page). Re-verified after: **encoded 12 / ambiguous 40 /
     open 10 / gap 0 over 62 units**, kind tally 2 Finding + 4 Observation + 3 Classification +
     1 Suggestion + 2 Assertion, ranks replay 62/0, kind replay 12/0 — the slice-5 pin, exact.
   - **Two doc corrections found while re-reading the definitions.** The reading guide in
     `onco-typed.esl` (and the `pins.tsv` note) labelled the verb axiom's FIRST argument the
     subject; transitive axioms are `obj -> subj -> Prop` (`dcg::rules::constructions`), so the
     first argument is the object («the exonuclease activity of WRN») and the second the subject
     (`kind_of(m)`). The pin note is emitted verbatim into the `DecisionPoint` rationale, so
     `claims-intact.esl` regenerated with it. The header's D6 naming argument, which reasoned
     from the old ternary arity, was restated for the binary one (the contrast with RO:0002215
     survives: these relate a MODEL to a gene with the process fixed, RO relates a gene to a
     process). `probe_restrictor_class_labels` generalized to any concept list
     (`EIGENIUS_PROBE_CUIS`) and to printing parents with labels — the instrument that answers
     "what is C1148824" (exonuclease activity, GO-derived, T045 Genetic Function).
   - **THE COMPOSED CONFIGURATION, measured for the first time** (raised in review: every number
     reported until now was a no-ranker floor). The plan puts selection INSIDE the discourse loop
     (§1.3, §2.2), but nothing measured it: the close-out hard-coded no ranker, and the sweep
     measures selection accuracy on ISOLATED sentences. `resolve_document_discourse_close_out`
     gained the `EIGENIUS_SELECTIONS` three-arm (exists → replay, absent + live → record, unset →
     none), the recorder that leaves the draw, and a selection-replay miss assert; the pinned
     tallies are now guarded on `selection_arm == "none"`, since with a ranker the pools collapse
     by design and asserting the tuple would assert the draw.

     | configuration | encoded | ambiguous | open | gap |
     |---|---|---|---|---|
     | discourse loop, NO ranker (the pinned floor) | 12 | 40 | 10 | 0 |
     | isolated sentences + ranker (the sweep) | 11 + 31 selected | — | 20 | 0 |
     | **discourse loop + ranker (the pipeline)** | **50** | **1** | **11** | **0** |

     Draws: `experiments/parsing/selections/2026-08-12-d67-discourse.json` (39 decisions),
     `experiments/parsing/kinds/2026-08-12-d67-discourse.json` (47 verdicts). REPLAY-VERIFIED —
     a second run reproduces 50/1/11/0 with ranks 62/0, selections 39/0, kinds 47/0. 49 claims
     land: 14 Finding, 13 Observation, 5 Classification, 3 Hypothesis, 2 Suggestion, 12
     Assertion.
   - **What the composed run cannot yet be gated on.** Scored against the 62-unit gold set, its
     39 decisions are 23 pinned-correct, 6 wrong, 1 abstained — and **9 unscorable**, because the
     pins were adjudicated on ISOLATED-sentence forests and a discourse-resolved reading has a
     different skeleton (the hole is replaced by its antecedent). So `selection_accuracy` as it
     exists gates the sweep, not the pipeline. Gating the composed configuration needs pins for
     resolved readings — new adjudication work, named here rather than papered over.
   - **d67 did NOT move selection accuracy.** A live draw over the replayed d67 forest reproduces
     the tracked baseline exactly: 31/31 chosen, 0 abstained, **21/31 reading-correct**, 23/31
     structure-correct, 0 invalid selected; parse metrics unchanged (226 readings, 144 skeletons,
     60/62 hits, gap 0). Recorded as `experiments/parsing/selections/2026-08-12-d67.json`. The
     suspicion that d67's inventory change had staled the baseline is refuted.
   - **A hypothesis raised and refuted by the data.** The demo's edited sentence has 120 readings
     carrying only 4 distinct glosses, and the chosen reading is gloss-identical to the
     pin-consistent one — suggesting the ranker is blind where the verbalizer collapses readings.
     Measured on the page draw: of the 6 wrong selections, **0** were gloss-indistinguishable
     from a correct candidate (57% of presented candidates carry a distinct gloss). Gloss
     degeneracy is real but is not what causes the page's selection errors; the demo sentence is
     a genuine single instance, not a general defect.
   - **Kind verdicts now carry their reasoning.** The live classifier asked the model for a
     `rationale` and dropped it — surfaced as a dead-field clippy error once
     `eigenius-reasoning/use-llm` was actually linted (the earlier "both configs" sweep never
     enabled it). `KindClassifier` returns `KindVerdict { kinds, rationale }` and the draw records
     it, abstentions included — an unmarked claim is unreferable and WHY it stayed unmarked is the
     reviewable part. `rationale` is optional on `KindRecord`, so draws recorded before the field
     replay unchanged (verified: the floor pin's `2026-08-12-reference.json` replays 12/0).
   - **Verified:** 173 workspace suites; clippy `-D warnings` on three configs (plain,
     encoding/use-llm, wordnet+encoding+reasoning/use-llm); `cargo fmt --all --check` clean; both
     DB-backed acceptance tests pass; `run.sh` end-to-end exit 0; the no-ranker floor pin
     (12/40/10/0, kinds 12/0) HOLDS.

## 8. Open question under review: is the claims ontology the right one? (2026-08-11)

Raised in review; the corpus answers half of it. What exists today:

- `reflection:` organizes claims by **epistemic source** — the class lattice
  `{Declared,Observed,Derived,Verified}Resource` (+ the trace classes that mint witnesses).
- `enc:EncodedClaim : reflection:DerivedResource` — ONE flat class for every pipeline-landed
  claim, beside the process records (`DiscourseUnit`/`DecisionPoint`/…).
- `reasoning:ReasoningSentence` — the gate-validatable certificate carrier.

Three findings:

1. **The taxonomy is one-axis, and discourse needs a second.** The page refers to claims by
   **discourse kind**: «these findings» (19/20/49), «These observations» (60), «These
   classifications» (45) — and it marks kinds on the way in («We **hypothesized** that…», 9/35).
   Epistemic source ≠ discourse kind: a hypothesis and a finding can both be Derived-landed,
   but «these findings» must NOT resolve to the hypothesis. So the §4 alignment as first drafted
   (`enc:EncodedClaim ⊑ finding-class`) is **unsound against the corpus** — it would type every
   claim as a finding. The revision direction: claims carry a **discourse/illocutionary kind**
   (finding / observation / classification / hypothesis / suggestion / assertion) as a class
   axis orthogonal to the epistemic-source lattice; per-kind alignment to the lexicon's noun
   classes is then 1:1 and semantically right, and kind assignment at landing has a concrete
   source (the matrix frame when the prose marks it; assertion as the default).
2. **"One claim" has no single class**, and the epistemic-source distinction is represented
   THREE ways: the class lattice, the `reflection:EpistemicStatus` enum, and the
   `JustificationTerm` constructor (`grade.rs` calls the grade "a structural projection…, not a
   stored field"). The per-source classes are load-bearing (each carries its REQUIRED fields:
   `declared_by`, trace shapes), so the revision is not "collapse to one class" — it is naming
   the missing ROOT (a claim/assertion superclass over the source lattice) and deciding which of
   the three representations is authoritative where.
3. **Process and content share `enc:` comfortably** — no pressure there; the revision is about
   the claim classes, not the pipeline-state records.

**Decision needed** (own design note before slice 5): the two-axis claim model — the kind
enumeration and its home (`enc:`? `reflection:`?), how kind interacts with the source lattice
and the graders, the set/group antecedent term, and the per-kind lexicon alignment layer's
placement (it must see both vocabularies, so it loads over the seeded lexicon — not bootstrap,
not the bare `encoding.esl` chain). Slices 2–4 proceed under the current shape; nothing in them
depends on the answer.
