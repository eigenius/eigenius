# D63 — next steps (method + stages order)

Short running to-do. Authoritative detail in the source docs: **Stages** = the document→encoding
pipeline (`d63-document-preprocessing-scope.md` §1); **reshape Phases** (`d63-kind-predication-reshape.md`
§6). Stage/Phase mapping: reshape Phase A = the tail of Stage B (done); Phase C = a slice of Stage C;
Phase B = off-timeline cleanup (done).

## Method (the plan)

1. **Phase 1 — build + test the whole pipeline ALGORITHM in Rust only, in-process.** Preprocess → parse
   → anaphora resolution, end to end. The LLM parts stay in Rust via **aLLM + `--features allms`**,
   exactly as the sense reranker (`AnthropicSenseRanker`) and the abbreviation proposer
   (`AnthropicAbbreviationProposer`) already do. Validate the algorithm before any service work.
2. **Phase 2 — once the algorithm works, refactor the LLM parts out into the orchestrator** (Deno/TS):
   the served gRPC path. Not before Phase 1 is validated.

## The pipeline abstraction (the anchor)

All the remaining work hangs off one contract — [`DocumentPipeline`](../../kernel/src/dcg/pipeline.rs),
`encode(document: &str) -> DocumentEncoding` (per-sentence `SentenceOutcome`:
`Encoded`/`Ambiguous`/`Open`/`Gap`; scope note §1a). Read each remaining item as **which of two
orthogonal axes it extends** — which is also what the two docs' "Phase" numbers *separately* count:

- **What it produces** (deepen the output) — the *scope-note* phasing. `SentenceOutcome::Encoded(Item)` →
  a **graded proposition** (reshape Phase C); `DocumentEncoding.glossary` → the full document-context
  family (figures/tables/citations). The caller reads richer output; the impls are untouched.
- **How it runs** (more impls, fixed seams) — the *Method* phasing above. Each impl is a choice of
  proposers (the `AbbreviationProposer` / `Proposer` seams) + where the doc layer lives:
  `InProcessPipeline` (in-memory, built) → a persistent in-process impl (full lexicon) → a served impl
  (Phase 2). **Phase 2 is a second `impl DocumentPipeline`, not a rewrite** — the contract is the seam.

So a grade/family item deepens the **output type** (`SentenceOutcome` / `DocumentEncoding`); a served or
persistent item is another **impl**; an LLM item swaps a **proposer seam**. The checkboxes below stay
grouped by the Method (run) phasing — this section is the lens to read them through.

## Done
- **Reshape Phase A** — `kind_of` axiom + unified `kind_raised_nps` (bare mass + plural, incl.
  compounds). Committed `04bab3d`; validated over full-UMLS re-measure (**OPEN 35 → 0**, any-parse ~61%).
- **Reshape Phase B** — retired the `Quantification` hole carrier; `EntityRef`/anaphora untouched;
  `d62-bare-plural-quantification.md` marked superseded. Green. *(uncommitted)*

## Phase 1 — the Rust algorithm (in order)
- [x] **Stage A · preprocess** — extract (Schwartz-Hearst) / ground / emit glossary aliases;
      `AnthropicAbbreviationProposer` (LLM, allms) for non-parenthetical defs; reusable
      `document_glossary_resources[_with]` seam. Built + in-memory tested (`abbreviation_pipeline_end_to_end`).
  - [x] **Validated on the DB corpus** (full UMLS, in-process, deterministic — `measure_abbreviation_glossary`):
        MSI/MMR/MSS/PARP-1 ground to real CUIs; bare `MSI` subjects recover GAP→**CLOSED** as
        `kind_of(C0920269)` (+ the "several cancers" compound-bare-plural → `kind_of(Σx:cancer.…)`).
        Residual: "Lynch syndrome" (named-disease item); `MMR` already parsed on base (glossary narrows).
- [x] **Stage B · parse** — chain the doc-glossary layer + `LexicalIndex` + parse. Built.
- [~] **Stage C · anaphora resolution (D64)** — in Rust.
  - [x] **The discourse resolve loop** `LexicalIndex::resolve_document(sentences, lemmatizer, proposer)` —
        parse each sentence, resolve `EntityRef` holes against the in-scope candidates (`resolve_with`,
        kernel re-gates), then harvest the sentence's entities (`entity_candidates`, most-recent-first)
        for later sentences. Fail-closed. Returns a `Vec<SentenceOutcome>` (`Encoded`/`Ambiguous`/`Open`/`Gap`
        — the classified per-sentence result, not a bare `Option<Item>`). Built + tested
        (`resolve_document_threads_discourse_across_sentences`); single-sentence + live-LLM paths already
        tested. The resolver primitives (`resolve_open`/`resolve_with`/`AnthropicProposer`) pre-existed;
        the candidate assembly + discourse threading is the new piece.
  - [x] **Reshape Phase C grade** — a closed prop → `epistemic:declared`. Built as
        `eigenius-reasoning::grade` (`ClaimGrader` trait + `DeclaredClaimGrader`): a parsed `Prop` →
        a **3-resource claim cluster** (the declaring `reflection:DeclaredResource` carrying
        `canonical_proposition`, its `DeclarationTrace` emitting the chain witness, the
        `reasoning:ReasoningSentence` with a `JustifiedBy.declared` certificate) → committed → the D39
        gate returns **`Holds`**. Tested (`grade.rs`, incl. fail-closed: drop the trace → `Fails`).
        The `reference:Citation` grade-climb (reshape §4 row 2) is the next increment (`Warrant` is
        `#[non_exhaustive]`).
  - [ ] Refinements: candidate surfaces = readable labels (not IRI local names); kinds/props as
        antecedents; intra-sentential binding; live-LLM `resolve_document` over a multi-sentence corpus slice.
- [~] **Phase-1 end-to-end harness** — one in-process Rust run: document text → glossary → parse →
      resolve → graded props, over the full lexicon. This is the "algorithm works" gate.
  - [x] **The pipeline contract + in-process impl** (`kernel/src/dcg/pipeline.rs`): the `DocumentPipeline`
        trait (`encode(&self, document: &str) -> DocumentEncoding`) with the input/output shape —
        `DocumentEncoding { glossary: Vec<AbbrDef>, sentences: Vec<SentenceEncoding{ text, outcome }> }` —
        and `InProcessPipeline`, which composes Stage A (glossary → in-memory doc layer) → Stage B+C
        (`resolve_document`). The LLM steps sit behind the proposer traits, so **Phase 2 swaps proposer
        impls without touching the contract**. Built + tested (`in_process_pipeline_encodes_a_document_end_to_end`,
        one `encode()` over the demo layer exercising all three stages). *(uncommitted)*
  - [x] **The grader + ingestion layer** (`eigenius-reasoning::{grade, ingest}`): `DocumentIngestion`
        trait + `InProcessIngestion` composes `DocumentPipeline` + `ClaimGrader` — encode → grade every
        `Encoded` sentence → commit the clusters onto the parsed doc chain → validate each through the D39
        gate, returning per-sentence outcome + claim + verdict. This **is** the harness, promoted from test
        code to a trait+impl. Tested (`ingest.rs`): `instability affects HeLa` → committed, `Holds`-validated
        `affects(kind_of(Instability), hela)`. The pipeline exposes its doc layer via an inherent
        `encode_with_layer` (kept off the trait so the served realization stays clean). *(uncommitted)*
  - [x] **D47 codec completion** (prerequisite, discovered here): `encode_type`/`resolve_const_ref` now
        round-trip a **term-level resource individual** (`Exp::EigonResource`) via `ConstRef(iri)`,
        discriminated on decode by the resolved resource's class — the third sibling of
        `EigonClass`/`EigonAxiom`. Without it, no parsed proposition naming an entity (`hela`) could be
        graded. Regression-swept: kernel + reasoning + statistics + schemaorg + lean all green. *(uncommitted)*
  - [ ] Remaining for the gate: a run over the **full lexicon** (DB-backed `base` needs a persistent doc
        layer, not the in-memory overlay — the `with_storage` seam noted in `pipeline.rs`), and the
        `reference:Citation` grade-climb.

## Phase 2 — orchestrator refactor (LATER; do not start until Phase 1 is validated)

> **A second `impl DocumentPipeline`** (the "how it runs" axis) — the trait/contract from Phase 1 is
> reused verbatim; only the proposer impls (→ orchestrator RPCs) and the doc-layer home (→ committed
> branch) change. If Phase 2 forces a change to the *contract*, that is a signal the seam was drawn wrong.

- [ ] Move the LLM steps (abbreviation extraction, sense rerank, anaphora proposal) out of the kernel
      into the orchestrator; expose the deterministic emission server-side.
- [ ] Served path: the commit+parse plumbing already exists and is **branch-aware** — `CreateBranch` →
      `Load(branch)` → `ParseSentence(branch=…)`, no kernel change for Stage B. The **missing** piece is
      text→grounded-`LexicalEntry` emission over gRPC (a thin RPC calling
      `extract_abbreviations`+`glossary_resources`, or the planned `orchestration/src/components/
      extract_document_structure.ts`, which does **not exist yet**).
  - Gotchas: branch names forbid `:` (use `doc-<id>`, not `doc:<id>`); the CLI `lexicon parse` has no
    `--branch` flag yet (`remote_parse` hardcodes empty branch); persistent backend required throughout.
- [ ] Figure / table / citation binding (`document:FigureRef`/`TableRef`/`reference:Citation`) —
      preprocessing-note Phase 2.

## Parse-gap closure (make the test document parse completely)

Full-lexicon baseline measured `2026-07-04` and triaged into an ordered plan:
**[d63-parse-gap-closure.md](d63-parse-gap-closure.md)**. Headline (62 units, deterministic): 39 AMBIG,
17 grammar-gap, 6 missing-lexeme, **0 encoded**, 0 open. Target: grammar-gap + missing-lexeme → 0.
Dominant fix = missing **verb + PP-complement frames** (~10 of 17; `occur in`/`arise from`/`contribute
to`/… — witnessed by `observe in` parsing vs `occurs in` gapping). Then the 4 OOV
(double-stranded/hypermutable/pcr-based/recq), `than`, `as a biomarker`, coordination/apposition, the
`are_kind` compound-subject edge, and named-disease (Lynch). See §4 for the step-by-step.

## Residuals (deferred to *after* the document parses — step 3+)
- [ ] Sense-crowding → clean single (`encoded`) parses — **0 encoded**: everything that parses is
      ambiguous (×8–×64) over full UMLS (diagnosis lever #2; the reshape does nothing for this).
- [ ] Long-sentence perf — Lever B beam drops millions of chart items on 16–21-tok units (3–5 min each).
