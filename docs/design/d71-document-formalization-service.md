# D71 — The document formalization service

*Status: design memo · `2026-08-17`. No code yet.*

*Supersedes [D62](d62-encoding-engine-prose-to-trees.md) §8, which assigned the generation half of
the encoding engine to the D14 institution protocol. This note reassigns it: formalizing prose is a
**service operation**, not an institution. The institution shape is reserved for the D61 faithfulness
half, which satisfies the D14 criteria that the generation half does not.*

*Depends on: [D14](d14-institution-realisation.md) (institutions), [D21](d21-task-traces-and-checkpointing.md)
(tasks), [D62](d62-encoding-engine-prose-to-trees.md) (the pipeline's stages), D63/D64/D66/D67/D68/D70
(the stages as built). This is the Stage-4 deliverable of
[parser-pipeline-plan.md](../notes/parser-pipeline-plan.md).*

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

Both are pure additions to the emitter and are the first build slice (§13), because every surface
depends on the handle.

---

## 5. Where things live

**The doc branch is a workspace, not a destination.** `with_storage(backend, doc_id)` builds the
doc-glossary layer on the persistent store and commits it to branch `doc-<doc_id>`, drop-and-recreate,
and *never advances the interactive chain*. That stays. The glossary resources also travel **inside**
the artifact, so an artifact loaded onto any other branch is self-contained.

**Landing is an explicit `Load`** of the artifact onto whatever branch the caller chooses. The service
does not decide where knowledge goes.

**Idempotency.** Same source + same recorded draws → byte-identical artifact: the emitter is
deterministic and `--timestamp` is a caller input for exactly this reason. Re-loading an unchanged
artifact hits the anchored-commit cache and reports `branch_advanced = false`.

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

**Resume is out of scope for v1.** The pipeline's live state is the discourse candidate set plus the
ranker's prior-selection context; restoring it mid-document is a checkpointing design of its own. A
cancelled run leaves its `doc-<id>` branch and is re-run. Stated here so the omission is a decision,
not an oversight.

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

Two record systems exist and they answer different questions:

| | Chain | Files |
|---|---|---|
| What | `enc:DecisionPoint` (authority, selected claim, candidate count, runner-up skeletons), `enc:AnaphorBinding` (authority, antecedent, confidence) | `ranks.json`, `selections.json`, `proposals.json`, `kinds.json` |
| Question answered | *What was chosen*, for a reader of the graph | *What the proposer was asked and what it said*, so a run reproduces without an LLM |
| Lifetime | As long as the claim | As long as the experiment |

Keep both; do not merge them. The service takes draw handles as **inputs** and emits `DecisionPoint`s
as **outputs**.

Two invariants that have already cost measurement time, restated so a surface author does not
rediscover them:

- A draw is keyed on the **presented pool**. A draw recorded against a different Stage-A glossary
  cannot answer this driver's questions (found `2026-08-12`).
- A replay with `misses > 0` is a **different experiment, not a reproduction**.

*Open (§14):* whether a service run should also write its draw to the chain, so a re-run is
reproducible from chain data alone. Deferred — a draw is a prompt-keyed transcript that changes with
the model, and the chain wants the decision, not the transcript.

---

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
  preceded it: the candidate set, the landed claims, and the ranker's prior selections. Two answers —
  re-run the whole document with the pin added (no new machinery, seconds-to-minutes per edit), or
  checkpoint the discourse state per unit (fast edits, new machinery, and the same state-restoration
  problem §6 deferred for resume). This is a design decision, not an implementation detail.
- **(b) A chain home for pins.** Today a pin is a TSV row read by a harness. A human override must be
  a chain-resident `DecisionPoint` with `authority_pin` on the doc branch, read back as the ranker's
  declared arm. That is a new input path into the pipeline — pins-from-chain, not pins-from-file.
- **(c) Supersession semantics.** A re-encoded claim is a *new* claim; its content hash differs. Either
  the old one is retracted or the structure is re-emitted wholesale. Re-emit wholesale is simpler and
  matches the drop-and-recreate lifecycle already in force.
- **(d) An editable cell.** Per-decision controls and per-unit re-run in the notebook.

**Scope call: separate effort.** (a) and (b) are both structural — one picks a discourse-state model,
the other changes how the pipeline reads its declared arm — and neither is needed to make the four
surfaces work. (c) and (d) follow from whatever (a) and (b) decide.

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

**Rewrite** [parser-pipeline-plan.md](../notes/parser-pipeline-plan.md) Stage 4 to this shape.

**Add**: `ReasoningStructure` + `reference:Reference` emission; `TaskKind`; the `FormalizeDocument`
RPC; the `eigenius_formalize_document` MCP tool; the `formalize` notebook cell type (bootstrap edit ⇒
batched reseed).

---

## 13. Build order

| # | Slice | Exit gate |
|---|---|---|
| 1 | Artifact root + provenance (`ReasoningStructure`, `enc:source_document` → `reference:Reference`) | `artifact_completeness` + `acceptance` green; demo v2 regenerates and still justifies twice / `Fails` on the edited variant |
| 2 | Declaration cleanup + doc amendments (§12) | `encoding_validates.rs` green; no dangling `enc_sig:` reference in the tree |
| 3 | `TaskKind` generalization | Existing task tests green; a formalize task appears in `ListTasks` with its own kind |
| 4 | `FormalizeDocument` RPC | E2E over the demo paragraph through the RPC produces an artifact **byte-identical** to the CLI's |
| 5 | MCP tool | `orchestration/tests/mcp_test.ts` covers start → poll → artifact |
| 6 | `formalize` notebook cell (read-only) | Playwright e2e; reseed done and the manifest pin updated in the same commit |

Slices 1–2 are independent of the rest and unblock everything.

---

## 14. Open questions

1. **Source transport** — inline text in the request, or a chain-resident source resource? Inline is
   simpler; chain-resident gives the `reference:Reference` a natural home and makes re-runs cite the
   same source.
2. **Does the `doc-<id>` branch survive a completed run?** It is a parse workspace and the artifact is
   self-contained, so deleting it is defensible; keeping it makes a re-run cheaper and a failure
   inspectable.
3. **Draw-to-chain** (§9) — should a service run record its LLM draws on-chain?
4. **Per-unit discourse checkpointing** (§11a) — needed for interactive override, and the same
   machinery that would give §6 resume.

---

## 15. References

- [D14](d14-institution-realisation.md) §1.1–1.2 (what is and is not an institution), §6 (query as the
  universal primitive), §9 (dispatch)
- [D21](d21-task-traces-and-checkpointing.md) (task records, suspension, cancellation)
- [D62](d62-encoding-engine-prose-to-trees.md) §8 (superseded), §9 (the gap stream as a product)
- [D64](d64-llm-anaphora-resolution.md) (S3 as a pipeline step, not an institution)
- [parser-pipeline-plan.md](../notes/parser-pipeline-plan.md) (Stages 1–3 as built)
- [d63-next-steps.md](../notes/d63-next-steps.md) Phase 2 (the seam test)
