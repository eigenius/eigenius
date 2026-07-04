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
  - [ ] **Reshape Phase C grade** — a closed prop → `epistemic:declared`; a `reference:Citation` witness
        climbs the grade. (Reasoning-layer integration, D39.)
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
  - [ ] Remaining for the gate: graded props (reshape Phase C) + a run over the **full lexicon** (DB-backed
        `base` needs a persistent doc layer, not the in-memory overlay — the `with_storage` seam noted in
        `pipeline.rs`).

## Phase 2 — orchestrator refactor (LATER; do not start until Phase 1 is validated)
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

## Residuals (separate; not blocking the pipeline)
- [ ] Sense-crowding → clean single (`encoded`) parses — everything parses ambiguous (×256) over full
      UMLS (diagnosis lever #2; the reshape does nothing for this).
- [ ] Residual grammar constructions — comparatives / `than` / "as a biomarker".
- [ ] OOV — hyphenated / `-based` (double-stranded, hypermutable, pcr-based, recq).
- [ ] Named-disease handling (Lynch syndrome — `cat_np` injection for named entities).
