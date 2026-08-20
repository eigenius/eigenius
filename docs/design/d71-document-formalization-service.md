# D71 — The document formalization service

*Status: design memo · `2026-08-17`; §5 branch lifetime and §9 draw home decided `2026-08-18`. No code yet.*

*Supersedes [D62](d62-encoding-engine-prose-to-trees.md) §8, which assigned the generation half of
the encoding engine to the D14 institution protocol. This note reassigns it: formalizing prose is a
**service operation**, not an institution. The institution shape is reserved for the D61 faithfulness
half, which satisfies the D14 criteria that the generation half does not.*

*Depends on: [D14](d14-institution-realisation.md) (institutions), [D21](d21-task-traces-and-checkpointing.md)
(tasks), [D62](d62-encoding-engine-prose-to-trees.md) (the pipeline's stages), D63/D64/D66/D67/D68/D70
(the stages as built). This is the fourth and last stage of the parser-pipeline build map
(approved `2026-08-11`, retired `2026-08-19` — Stages 1–3 are built, their design notes stand, and
their as-built record is work-stack entry 0). D71 is the surviving spine document for this work.*

---

## 0. The decision

Prose formalization is a **service operation** over the existing
[`DocumentPipeline`](../../kernel/src/dcg/pipeline.rs) contract. It produces a **resource-set
artifact**, runs as a **task**, and is reachable from four surfaces — CLI, gRPC, MCP, notebook — each
of which is a thin driver. No new pipeline logic lives in any surface.

| | D62 §8 said | D71 says |
|---|---|---|
| Shape | A D14 institution (`enc:FormalizeDocument`) | A service operation over `DocumentPipeline` |
| Invocation | `FIBER … INTO` (synchronous EigenQL) | A task, started by RPC, polled by task status |
| Output | One `EncodedClaim` resource per dispatch | One artifact (resource set), rooted at an `enc:ReasoningStructure` |
| Commitment | The FIBER commit cycle | An explicit `Load` of the artifact — generation and commitment stay decoupled |
| Runtime | `runtime: external` (orchestration-hosted) | No institution runtime; the parser is kernel-side and the proposers are the outbound seam |
| Reproducibility | Draw files beside the source | Draws committed to the `doc-<id>` working branch — a re-run replays from the chain, and the branch is prunable |
| Institution shape | Generation (D62) + verification (D61) | Verification (D61) only |

§1 argues the negative, §2–§9 specify the positive, §10 states what D61 inherits, §11 scopes the
human-override loop, §12 lists what changes in the tree.

---

## 1. Why this is not an institution

### 1.1 It fails three of D14's four criteria

D14 §1.2 states what an institution is. Applied to the encoding engine:

| D14 §1.2 criterion | Encoding engine |
|---|---|
| "Has its own notion of well-formedness — a satisfaction relation expressible as functions over typed Resources" | **No.** It borrows the kernel's: structural validation plus the parse-time felicity re-gate. D14 §1.1 excludes both from the protocol — the validator is the switchboard, not a fibre. |
| "Produces results with internal structure — morphisms within its fibre" | **No.** There is no encoding fibre. `enc:EncodedClaim` is a `reflection:DerivedResource` ([encoding.esl:356](../../ontologies/encoding/encoding.esl#L356)); the claims land in the reasoning fibre and are graded there. |
| "Can answer queries about its own results" | **No.** |
| "Registers by committing typed declarations and providing a runtime" | Yes — but this is true of any component that ships an ontology. |

### 1.2 The precedent in the tree points the same way

Every resource declaring `institution:Institution` today *decides* something: the five Julia solvers
(Catalyst, DiffEq, Symbolics, JuMP, Intervals) solve, [Lean](../../ontologies/lean/lean-institution.eigon.json)
checks proofs, [statistics](../../ontologies/statistics/statistics.esl) recomputes a test from raw
replicates, [reasoning](../../ontologies/reasoning/reasoning.esl) validates a justification
certificate, runtime-substrate registers environments.

Meanwhile five importers — UMLS, WordNet, NCBI-gene, obograph, schema-org — all do *external source →
typed resource set → `Load`*, and not one of them is an institution. The parsing pipeline is
structurally the sixth importer, distinguished only by having an LLM in the loop and a kernel re-gate
on its output.

### 1.3 D62 §8's four reasons dissolve on inspection

| D62 §8 claim | Where the property actually comes from |
|---|---|
| "Generation = OnDemand" | A statement about *invocation*, not about institution-ness. |
| "Derived by construction — an institution dispatch emits a `DerivedResource` under `ProgramTrace → IsDerivedAs`" | `DerivedClaimGrader` produces that cluster with no institution involved. The property comes from the grader plus the reflection ontology. |
| "Felicity = AutoOnLoad" | The felicity gate is *structural validation* — kernel, explicitly not an institution (D14 §1.1). |
| "Faithfulness = a separate verification institution (D61)" | **Holds.** See §10. |

D62 §8 wrote the generation and verification halves as one section and gave both the institution
label. Only the second earns it.

### 1.4 What is given up, honestly

- **`ListInstitutions` discoverability.** Replaced by an RPC and an MCP tool — which is where the
  agent-facing and human-facing surfaces actually look.
- **FIBER invocability from EigenQL.** Independently the wrong shape: FIBER dispatch is synchronous
  inside query evaluation, binds exactly one response resource per row, and rejects a second arrival
  at the same `INTO` IRI ([fiber.rs](../../kernel/src/query/evaluate/fiber.rs)). A document
  formalization is minutes long and produces a resource *set*.
- **Comorphism declarability.** A `Comorphism` needs an `ExportFormat`, which needs an
  `institution_ref`. So `enc:EncodedClaim → reasoning:ReasoningSentence` cannot be declared as a
  comorphism and stays Rust inside `DerivedClaimGrader`. This is a real loss; §10 says when the door
  reopens.

---

## 2. The shape

```
                          ┌──────────────────────────────┐
   CLI ─────────┐         │  DocumentPipeline::encode    │
   gRPC ────────┼───────▶ │    Stage A  glossary         │ ──▶  artifact  ──▶  Load
   MCP  ────────┤         │    Stage B  parse + select   │      (ESL /         (validator,
   notebook ────┘         │    Stage C  resolve + land   │       Eigon-JSON)    Rule 22, D39 gate)
                          └───────────┬──────────────────┘
                                      │ seams (trait objects)
                     AbbreviationProposer · CategoryProposer · ReadingRanker
                     Proposer (anaphora) · ClaimLander · parser_setup
```

**One contract, one artifact, many surfaces.** Each surface chooses proposer impls, an artifact
destination, and a set of replay handles. None of them owns pipeline logic. The rule from
[d63-next-steps.md](../notes/d63-next-steps.md) §Phase 2 stands as the test: *if a surface forces a
change to the contract, the seam was drawn wrong.*

---

## 3. The contract

Unchanged from what Stages 1–3 built ([pipeline.rs](../../kernel/src/dcg/pipeline.rs)):

```rust
pub trait DocumentPipeline {
    fn encode(&self, document: &str) -> Result<DocumentEncoding, PipelineError>;
}

pub struct DocumentEncoding {
    pub augmentation: LexiconAugmentation,   // Stage-A entries + residual OOV
    pub sentences: Vec<SentenceEncoding>,    // one per body sentence, in order
}

pub struct SentenceEncoding {
    pub text: String,
    pub outcome: SentenceOutcome,            // Encoded | Ambiguous | Open | Gap
    pub selection: Option<SelectionOutcome>, // → enc:DecisionPoint
    pub resolution: Option<ResolutionOutcome>, // → enc:AnaphorBinding
}
```

Parse-level failure is a per-sentence `SentenceOutcome`, never an error: a document with gaps still
encodes, honestly. The only `PipelineError` is a doc-layer commit failure.

The service adds a **home** for this contract and a **transport** for its seams. It adds no stage.

---

## 4. The artifact

`emit_document` ([emit.rs](../../crates/eigenius-encoding/src/emit.rs)) already produces, per document:
the Stage-A glossary resources, and per unit a `DiscourseUnit` + `ScopedUnit` + the Derived claim
cluster (`enc:EncodedClaim` + `reflection:ProgramTrace`) + `enc:DecisionPoint` + `enc:AnaphorBinding`s,
plus an `enc:CutItem` for every unit that did not encode. Output format is Eigon-JSON or ESL; the ESL
printer is the inverse of the loader.

**Generation stays decoupled from commitment.** The artifact is inspectable and diffable, and
committing it means loading it through the kernel — validator, Rule 22, the D39 gate — exactly as
[demo/prose-to-formulas-v2](../../demo/prose-to-formulas-v2/run.sh) does. Keep this. It is what makes
a formalization reviewable before it enters the graph.

### 4.1 Two gaps this note closes

**No root resource.** `enc:ReasoningStructure` is declared ([encoding.esl:413](../../ontologies/encoding/encoding.esl#L413))
and never emitted. Without it the artifact has no handle: nothing for the RPC to return, nothing for
the notebook cell to re-open, nothing for a later run to supersede. The service emits it, with
`enc:claims` listing the encoded claims and `enc:document` naming the source.

**No bibliographic provenance.** `enc:source_document → reference:Reference` is declared and never
set; `discourse_unit` instead writes `"<path> (sha256 <hex>)"` into `enc:section`, a free-text field
meant for `"Results §2.1"`. The service mints (or accepts) a `reference:Reference` for the source and
sets `enc:source_document` on every unit, leaving `enc:section` for what it is for.

The `reference:Reference` travels **inside** the artifact, like the glossary does. It has to: the
artifact's units reference it, and Rule 22 is closed-world over same-or-lower layers, so a Reference
left behind on the working branch would dangle the moment the artifact is loaded anywhere else. This
holds whichever way §14.1 settles source transport.

Both are pure additions to the emitter and are the first build slice (§13), because every surface
depends on the handle.

---

## 5. Where things live

**The doc branch is the run's working record, not a destination.** `with_storage(backend, doc_id)`
builds the doc-glossary layer on the persistent store and commits it to branch `doc-<doc_id>`,
drop-and-recreate, and *never advances the interactive chain*. That stays. What the branch holds
grows: the doc glossary **and** the run's proposal draws (§9). The glossary resources also travel
**inside** the artifact, so an artifact loaded onto any other branch is self-contained.

**The branch survives the run and is prunable** (decided `2026-08-18`). It is what makes a re-run
LLM-free and a failed run inspectable; when neither is wanted any more, `DeleteBranch` drops it. What
is lost on pruning is the transcript and therefore free replay; what survives is every decision, because
the decisions are in the artifact and the artifact was loaded elsewhere. Pruning is a caller action —
the service never deletes a branch it did not create in that run.

**Landing is an explicit `Load`** of the artifact onto whatever branch the caller chooses. The service
does not decide where knowledge goes.

**Idempotency.** Same source + same draws → byte-identical artifact: the emitter is deterministic and
`--timestamp` is a caller input for exactly this reason. With the draws on the branch (§9), "same
draws" is the default for a re-run against the same `doc_id` rather than something the caller has to
arrange. Re-loading an unchanged artifact hits the anchored-commit cache and reports
`branch_advanced = false`.

---

## 6. The task lifecycle

Formalizing a document takes minutes and N LLM round-trips. Three of the four surfaces cannot hold a
synchronous call open for that: MCP has no long-call idiom (its tools are one-shot RPC wrappers), the
notebook needs progress and cancel, and a CI driver wants to poll. So the document-level operation is
**asynchronous**, and the D21 task machinery is what already exists for that —
`ListTasks` / `GetTaskStatus` / `CancelTask` are live RPCs and are already exposed over MCP.

**The one change D21 needs.** `TaskRecord` ([task/mod.rs:116](../../kernel/src/task/mod.rs#L116)) is
program-bound: `program_iri` + `input_iri`. A formalization is not a program run. Two ways to proceed:
mint a synthetic program IRI and leave `input_iri` empty, or give the record a kind. Take the kind —
a fake program IRI would make `ListTasks` lie to every reader, and the lie would have to be special-cased
in the notebook, the MCP tool, and the CLI:

```rust
pub enum TaskKind {
    ProgramRun { program_iri: String, input_iri: String },
    Formalize  { doc_id: String, source_sha256: String },
}
```

`TaskInfo` gains the discriminant; the existing fields stay for `ProgramRun`.

**Progress** is per-unit: `units_total` / `units_done` plus the current stage. The discourse loop is
already sequential and per-sentence, so this costs a counter.

**Cancellation** is cooperative at the sentence boundary — the same granularity the loop already
commits at.

**Resume is out of scope for v1, and cheaper than it looked.** The pipeline's live state is the
discourse candidate set plus the ranker's prior-selection context. Serializing that state is a
checkpointing design of its own — but with the draws on the working branch (§9), it does not have to
be serialized to be recovered: re-running units 1..k against the same pinned base, the same committed
glossary, and the same committed draws is deterministic and LLM-free, and reconstructs the state
exactly. Resume becomes *replay the prefix, then continue live*, which needs no new state format.
What remains is compute — re-parsing the prefix — so v1 still re-runs from the start and the
optimisation is deferred with a known shape rather than an unknown one.

---

## 7. The surfaces

### 7.1 gRPC — `FormalizeDocument`

The precedent is ours and recent: D63/D65 made parsing a **service operation**, `ParseSentence`, not
an institution query. This is its document-level sibling and should reuse its conventions verbatim —
`branch` / `at_layer` pinning, `scope` (lexicon IRIs) / `profile` selection.

```
FormalizeDocument(
  source_text | source_iri,     // inline prose, or a chain-resident source
  doc_id,                       // names the doc-<id> workspace branch
  branch | at_layer,            // what to parse over
  scope[] | profile,            // D65 lexicon scope
  draws { ranks, selections, proposals, kinds },  // replay handles; absent ⇒ live
  strict,                       // default false — see §8
  format                        // Esl | EigonJson
) -> task_id
```

The completed task exposes the artifact and the `enc:ReasoningStructure` IRI. Committing is a separate
`Load` by the caller.

A second operation, **`FormalizeUnit`** — re-encode one unit inside an existing `ReasoningStructure` —
is what §11 needs. It is specified there, not here, because its context requirement is the open
question of that section.

### 7.2 CLI

[`prose-to-esl`](../../crates/eigenius-encoding/src/pipeline.rs) is already a thin driver over
`DocumentPipeline` and is the reference implementation of this note. Its replay arms (`--pins`,
`--selections`, `--ranks`, `--proposals`, `--kinds`) stay exactly as they are — they are the
deterministic gate arms that the parse-rate sweep and the demo depend on.

The only change: given a server it drives the RPC; given `--snapshot` it stays in-process. Same
artifact either way, which is the acceptance gate in §13.

### 7.3 MCP

The MCP server's rule is one tool per kernel RPC ([mcp/server.ts](../../orchestration/src/mcp/server.ts)).
`eigenius_formalize_document` returns a task id; `eigenius_get_task_status` already exists to poll it;
the artifact returns as text the agent can read, diff, or hand to `eigenius_load`.

This surface is why §6 is not a preference. There is no way to express a minutes-long synchronous
call here.

### 7.4 Notebook

A new cell type, `formalize`, alongside `esl` / `eigenql` / `typescript` / `program-run` / `chart`.

**Cost note:** the notebook ontology is in `BOOTSTRAP_CHAIN`, so adding a cell type moves the
`notebook` hash, fails [bootstrap_manifest_pinned.rs](../../kernel/tests/bootstrap_manifest_pinned.rs),
and invalidates every snapshot on disk. Batch it with any other pending bootstrap edit and pay one
reseed, following that test's panic message.

The cell **holds**: source prose, `doc_id`, arm configuration, and the resulting `ReasoningStructure`
IRI. It **renders**: the per-unit table (ordinal · text · outcome · claim · verbalization), the
`CutItem` / `LexicalGap` stream, the `DecisionPoint`s with their runners-up, and the
`AnaphorBinding`s. It **runs** through the task path, so `TasksPanel` supplies progress and cancel
with no new UI.

Read-only in v1. Editing a decision is §11.

---

## 8. Outcomes, gaps, and what actually gates

There is no `Verdict` role. Three checks gate, none of them owned by this service:

1. **Structural validation** — kernel, on `Load` of the artifact.
2. **The felicity re-gate** — kernel, inside the parse: every proposed tree is type-checked before it
   is a reading at all. This is the trust boundary the LLM steps sit behind.
3. **The D39 justification gate** — `reasoning:qc_validate_justification`, `AutoOnLoad` on every
   `ReasoningSentence` commit. Institution-owned, and already live.

The service's own reporting is the per-unit outcome plus the gap stream, which D62 §9 calls a
first-class product rather than a failure log: each `CutItem` and `LexicalGap` names a construction or
word the corpus needed.

**Default: record, do not abort.** The CLI's `--partial` semantics become the service default — a unit
that does not encode lands as its `DiscourseUnit` + a `CutItem` naming the reason. An interactive
surface needs the three failing units named, not an aborted run. The strict arm stays available and
stays what CI uses; a pin that *contradicts* an encoded reading still aborts in both modes, because
that is pin drift, not a coverage gap.

**Known missing:** the `Fails` diagnostic from `qc_validate_justification` is not surfaced through the
load path. Unchanged by this note; still worth fixing where the notebook will show it.

---

## 9. Where recorded decisions live

Three records, three questions, three homes.

| | Decision | Draw | Experiment draw |
|---|---|---|---|
| What | `enc:DecisionPoint` (authority, selected claim, candidate count, runner-up skeletons), `enc:AnaphorBinding` (authority, antecedent, confidence) | `enc:ProposalDraw` — the question put to a proposer and the answer it gave, verbatim | `ranks.json`, `selections.json`, `proposals.json`, `kinds.json` |
| Question answered | *What was chosen*, for a reader of the graph | *What the proposer was asked and said*, so this document re-runs without an LLM | The same, across runs, corpora and model versions |
| Home | The artifact → wherever it is loaded | The `doc-<id>` working branch | The experiment directory |
| Lifetime | As long as the claim | Until the branch is pruned | As long as the experiment |

The decision and the draw are not the same record and neither subsumes the other. A `DecisionPoint`
says reading 3 was chosen by the ranker with these runners-up; it does not carry the pool as
presented, the prior-selection context, or the rationale text, and it does not exist at all for sense
ranking or discourse-kind classification. Reproducing a run needs the draw; reading the graph needs
the decision.

### 9.1 Draws live on the working branch

**Decided `2026-08-18`.** A service run commits its draws to `doc-<id>` as it makes them, alongside
the glossary layer that run built. This is what makes the earlier deferral unnecessary: the objection
was that the chain wants the decision, not the transcript — but the transcript is not going on the
branch the claims land on. It goes on the working branch, which is exactly where a run's scaffolding
belongs, and which is prunable (§5).

What it buys:

- **A re-run reproduces from chain data alone.** Point a run at the same `doc_id` and the draws are
  already there; no draw files, no key, no LLM. Today that requires four JSON files travelling beside
  the source.
- **The pool and the draw are pinned together.** A draw is keyed on the *presented pool*, and the pool
  is a function of the Stage-A glossary. On the branch, both are committed by the same run, so the
  consistency the key enforces is structural rather than a filename convention. The failure of
  `2026-08-12` — a draw recorded against a different Stage-A glossary, replayed against this pipeline
  and answering a different question — is not expressible in this arrangement.
- **A failed run is inspectable.** The draws up to the failure point are on the branch.

The invariant that does not change: **a replay with `misses > 0` is a different experiment, not a
reproduction.** Chain-resident draws miss on exactly the same conditions file draws do, because the
key is the same key.

### 9.2 The vocabulary

One class in `encoding.esl` (not bootstrap — no reseed), covering all four seams, because the four
Rust record types share one envelope: a keyed question and the answer to it.

```esl
class enc:ProposalDraw {
    description = "One recorded proposer exchange on a formalization run: the exact question put to an
                   untrusted proposer and the answer it gave. Committed to the run's doc-<id> working
                   branch, never to the branch the claims land on. Replay reads these back in place of
                   the draw files; a changed question MISSES, it never silently replays.";
    requires enc:draw_seam, enc:draw_key, enc:draw_question, enc:draw_answer;
    recommends enc:draw_unit, enc:draw_model, reflection:timestamp;
}
```

`enc:draw_seam` is a closed enumeration of individuals — `sense_rank`, `reading_selection`,
`anaphora`, `discourse_kind` — in the style of `enc:SelectionAuthority` and `enc:CutKind`, not a
string. `enc:draw_unit` points at the `DiscourseUnit` the exchange belongs to, so a per-unit re-run
(§11) can find its own draws.

`enc:draw_record` holds the serialized record — deliberately a transcript rather than a modelled
structure. (This note first specified a `draw_question` / `draw_answer` PAIR; implementation showed
that splitting them contradicts the very reason given for not modelling the record, since the split
is itself a restatement of the contract, and the four seams do not divide the same way — `RankRecord`
answers with a permutation per word, `SelectionRecord` with a chosen index plus rationale plus
runners-up. One field, one source of truth.) The Rust record types (`RankRecord`, `SelectionRecord`, and their
siblings) **are** the replay contract; the key function reads them field by field. Modelling them a
second time in ESL would create two definitions of one contract with no mechanism keeping them in
step, and the first divergence would be a silent replay of the wrong answer. The chain field is the
serialization of the contract, not a restatement of it.

### 9.3 Reading them back

The pipeline gains a draw source that is a branch rather than a set of files. The file arms stay
exactly as they are — they are what the parse-rate sweep, the demo, and CI run on, and they are how
a draw is compared *across* runs, which a per-document branch cannot answer.

Note for §11: this is the same shape as the chain-resident pin that the human-override loop needs
(§11b). Building draws-from-chain builds most of pins-from-chain.

## 10. What D61 inherits

D61 faithfulness verification satisfies all four D14 criteria: it has its own satisfaction relation
(does back-translation recover the source?), it produces a verdict, it answers about its own results,
and it gates. It declares an `Institution`, an `AutoOnLoad` `QueryClass` bound to `enc:EncodedClaim`
returning `institution:Verdict`, and promotes Derived → Verified.

None of that requires the generation half to be an institution — the QueryClass is declared by D61 and
bound to the class this service emits.

D61 is also what reopens the door §1.4 closed: once an institution owns `enc:EncodedClaim`, an
`ExportFormat` over it can anchor a `Comorphism` for `EncodedClaim → reasoning:ReasoningSentence`,
making the D39 hand-off inspectable and commit-time type-checked instead of implicit in
`DerivedClaimGrader`.

---

## 11. The human-override loop

The loop, concretely:

1. The notebook renders a `DecisionPoint` with its runners-up.
2. The human picks a different reading. The pick is a **pin** — `enc:authority_pin` already exists as
   a `SelectionAuthority` individual, and `PinReadingRanker` already implements the declared arm.
3. That unit is re-encoded with the pin in force.
4. The re-encoded claim supersedes the old one; a new artifact is produced and loaded.

**The crude version already works.** Add the pin to a pins file, re-run the whole document through
`prose-to-esl --pins`, reload. So the capability exists from day one; what is missing is that it be
per-unit and interactive.

What per-unit-and-interactive additionally requires:

- **(a) Discourse context for a single unit.** Re-encoding sentence 12 alone needs the state that
  preceded it: the candidate set, the landed claims, and the ranker's prior selections. The §9
  decision changes the shape of this: with the draws on the working branch, re-running units 1..11 is
  deterministic and LLM-free, so the choice is no longer "serialize the discourse state or don't" but
  "how much of the prefix to recompute per edit". The cheap version — replay the prefix, re-encode
  unit 12 under the pin — costs parse time only and needs no new state format. Whether that is fast
  enough for an interactive surface is a measurement, not a design question.
- **(b) A chain home for pins.** Today a pin is a TSV row read by a harness. A human override must be
  a chain-resident `DecisionPoint` with `authority_pin` on the doc branch, read back as the ranker's
  declared arm. That is a new input path into the pipeline — pins-from-chain, not pins-from-file.
- **(c) Supersession semantics.** A re-encoded claim is a *new* claim; its content hash differs. Either
  the old one is retracted or the structure is re-emitted wholesale. Re-emit wholesale is simpler and
  matches the drop-and-recreate lifecycle already in force.
- **(d) An editable cell.** Per-decision controls and per-unit re-run in the notebook.

**Scope call: separate effort** — but a smaller one than when this section was first written. The §9
decision dissolves most of (a) (prefix replay replaces a state format) and most of (b) (draws-from-chain
is the same input path as pins-from-chain). What is left is a measurement, a second reader on that
input path, and the UI. Still out of scope for v1, because none of it is needed to make the four
surfaces work, and (c)'s supersession semantics deserve their own decision.

Two v1 hooks keep the separate effort cheap, and both are already in this note for other reasons:
emit the `enc:ReasoningStructure` root (§4.1), so a structure is addressable and a superseding run has
something to point at; and specify `FormalizeUnit` (§7.1) as the operation the loop will drive, even
if v1 implements it as "re-run the document with this pin".

---

## 12. Consequences in the tree

**Delete** from [encoding.esl](../../ontologies/encoding/encoding.esl): `enc:FormalizeDocument` and
`enc:qc_formalize_unit` (lines 435–459). They are wired to nothing — `enc_sig:formalize_unit` has no
handler anywhere in the tree — and leaving them is the same class of dangling placeholder that file's
own comment already warns about, one level up. The `enc:` *vocabulary* is untouched and entirely
needed.

**Amend** [D62](d62-encoding-engine-prose-to-trees.md) §8: generation is a service; the institution
reading belongs to D61. Also §10 item 6 ("S8 institution wrapper"), §11.3's "insert into
`BOOTSTRAP_CHAIN`" note (the declarations are being deleted, not bootstrapped), and §11.5's
"`FormalizeDocument` pipeline institution" phrasing.

**Done `2026-08-19`:** `docs/notes/parser-pipeline-plan.md` deleted rather than rewritten — Stages
1–3 are built and recorded (per-stage notes + work-stack entry 0), and this note is Stage 4, so the
map had one live section and it was the wrong one. Its inbound references were re-pointed at the
per-stage notes and here.

**Add**: `ReasoningStructure` + `reference:Reference` emission; `enc:ProposalDraw` + its seam
enumeration (§9.2, `encoding.esl`, no reseed) and the draws-from-chain reader beside the file arms;
`TaskKind`; the `FormalizeDocument` RPC; the `eigenius_formalize_document` MCP tool; the `formalize`
notebook cell type (bootstrap edit ⇒ batched reseed).

---

## 13. Build order

| # | Slice | Exit gate |
|---|---|---|
| 1 ✅ | Artifact root + provenance (`ReasoningStructure`, `enc:source_document` → `reference:Reference`) | `artifact_completeness` + `acceptance` green; demo v2 regenerates and still justifies twice / `Fails` on the edited variant |
| 2 ✅ | Declaration cleanup + doc amendments (§12) | `encoding_validates.rs` green; no dangling `enc_sig:` reference in the tree |
| 3 ✅ | `enc:ProposalDraw` + commit draws to `doc-<id>` + draws-from-chain reader | A recorded run re-runs from the branch alone with `misses == 0` and emits a byte-identical artifact; the file arms still replay unchanged |
| 4 | `TaskKind` generalization | Existing task tests green; a formalize task appears in `ListTasks` with its own kind |
| 5 | `FormalizeDocument` RPC | E2E over the demo paragraph through the RPC produces an artifact **byte-identical** to the CLI's |
| 6 | MCP tool | `orchestration/tests/mcp_test.ts` covers start → poll → artifact |
| 7 | `formalize` notebook cell (read-only) | Playwright e2e; reseed done and the manifest pin updated in the same commit |

Slices 1–2 are independent of the rest and unblock everything. Slice 3 is independent of the surfaces
and is what makes slice 5's byte-identity gate cheap to run repeatedly.

**Slices 1–2 DONE `2026-08-19`.** The emitter takes a `DocumentMeta` (ns, path, sha256, timestamp,
optional `source_ref`) and emits the `reference:Reference` first and the `enc:ReasoningStructure` last
— last so every claim IRI it lists is already defined above it. A cited `source_ref` is pointed at,
never re-minted, so Rule 22 does the verifying. `enc:section` no longer carries `"<path> (sha256
<hex>)"`; the pair moved to `enc:source_path` / `enc:source_sha256` on the root, and the CLI gained
`--source-ref`. Gates: `artifact_completeness` loads the root and the Reference through the real
chain; the §3.5 `acceptance` run is `Holds` / `Fails` with the diagnostic; `run.sh --reparse` exits 0,
intact COMMITTED and edited REJECTED; workspace tests and `RUSTFLAGS="-D warnings" cargo clippy` are
clean.

**Slice 3 DONE `2026-08-19`, with one scope correction.** `enc:ProposalDraw` + the closed
`enc:DrawSeam` enumeration are in `encoding.esl`; `kernel/src/dcg/draw.rs` turns a seam's recorded
exchanges into resources (IRI content-addressed on `(seam, key)`, so an identical re-record is the
same resource and the branch does not grow), reads them back index-driven off a layer, and commits
them onto `doc-<id>` on top of the glossary layer. All four seams gained `to_json` / `keyed_draws` /
`from_json` beside their existing file `write` / `load`.

Extracting the record-side key removed a real hazard rather than just enabling this: `ReplaySenseRanker::load`
had been RE-DERIVING `rank_key` inline, with a comment asking the reader to keep the two copies in
step. It is one `record_key` function now, shared by the loader and the draw emitter — a third copy
would have been where they diverged, and a divergence there does not fail loudly, it MISSES, and a
miss falls back to seed order and reports itself as a reproduction.

*Scope correction:* the exit gate is met at the MECHANISM level (`kernel/tests/proposal_draw_round_trip.rs`
— record → resources → validated layer → read back → replay, 0 misses, seam-filtered), not end to end
through a surface. `prose-to-esl` opens a DISPOSABLE working copy of the snapshot and discards it on
exit, so no branch it writes can outlive the run; the CLI's replay arm is files, and that is right for
a copy-and-discard driver. Draws-on-branch is a property of a run against a LIVE store, so the
end-to-end arm lands with slice 5's RPC, which `commit_draws` and `draws_from_layer` are already
shaped for. §13's "independent of the surfaces" anticipated this; the gate wording did not.

One defect surfaced and fixed while regenerating: the old `enc:section` string embedded the
INVOKER'S ABSOLUTE PATH (`/home/hm/src/eigenius/...`), so the committed artifact had never been
reproducible off this machine — §5's byte-identity claim was false in a way no test looked at. The
demo now runs `prose-to-esl` from the repo root with a repo-relative `--source`. The emitter cannot
fix this itself (it has no repo root); `enc:source_sha256` is what actually pins the bytes, and
`enc:source_path` is caller-supplied text.

---

## 14. Open questions

*Answered `2026-08-18`: the `doc-<id>` branch survives a completed run and is prunable (§5); a run
records its LLM draws to that branch (§9.1). Per-unit discourse checkpointing (§11a) is no longer a
design question — prefix replay covers it — leaving a measurement, listed below.*

1. **Source transport** — inline text in the request, or a chain-resident source resource? Inline is
   simpler; chain-resident gives the `reference:Reference` a natural home on the working branch and
   makes re-runs cite the same source. Either way the Reference is emitted into the artifact (§4.1).
2. **Draw commit granularity** — one layer per unit, or one draw layer at the end of the run? Per-unit
   is what makes a failed run inspectable and the prefix replayable, and costs one layer per sentence
   on the working branch; per-commit cost is known to grow with chain length, which is tolerable for a
   page and not obviously so for a full paper. Measure before fixing the policy.
3. **Pruning policy** — manual `DeleteBranch` only, or a retention hint the service records for
   [D44](d44-automatic-data-lifecycle-management.md) to act on? v1 is manual; the question is whether
   a working branch should carry its own expiry.
4. **How much prefix replay is fast enough** (§11a) — a measurement on the WRN page: re-parse cost for
   a k-unit prefix with all draws replayed, which sets whether interactive per-unit override needs
   anything beyond replay.

---

## 15. References

- [D14](d14-institution-realisation.md) §1.1–1.2 (what is and is not an institution), §6 (query as the
  universal primitive), §9 (dispatch)
- [D21](d21-task-traces-and-checkpointing.md) (task records, suspension, cancellation)
- [D62](d62-encoding-engine-prose-to-trees.md) §8 (superseded), §9 (the gap stream as a product)
- [D64](d64-llm-anaphora-resolution.md) (S3 as a pipeline step, not an institution)
- Stages 1–3 as built: [d63-reading-selection.md](../notes/d63-reading-selection.md),
  [d64-demonstratives-as-holes.md](../notes/d64-demonstratives-as-holes.md),
  [d67-pipeline-unification.md](../notes/d67-pipeline-unification.md),
  [d68-claim-kinds.md](../notes/d68-claim-kinds.md), and work-stack entry 0
- [d63-next-steps.md](../notes/d63-next-steps.md) Phase 2 (the seam test)
