# Design Documents

This directory contains the architecture specification, implementation plan, and design documents for Eigenius.

## Core Documents

- **[architecture-v0.3.md](architecture-v0.3.md)** — The current architecture specification. The authoritative reference for all design decisions.
- **[implementation-plan.md](implementation-plan.md)** — Six-phase build plan with deliverables, test plans, and design document requirements.
- **[architecture-v0.2-review.md](architecture-v0.2-review.md)** — Design review of the v0.2 architecture, identifying contradictions and gaps (most resolved in v0.3).

## Design documents

The per-subsystem design notes, in number order. New documents are added here as
`d{N}-{short-name}.md`.

| # | Document |
|---|----------|
| D1 | [Eigon serialization format](d1-eigon-serialization-format.md) |
| D2 | [EigenQL v1 specification](d2-eigenql-specification.md) |
| D3 | [Program model and component interface](d3-program-model.md) |
| D4 | [Storage key encoding](d4-storage-key-encoding.md) |
| D5 | [gRPC API specification](d5-grpc-api-specification.md) |
| D6 | [Execution architecture and durability](d6-execution-architecture.md) |
| D6b | [Reasoning trace schema](d6b-reasoning-trace-schema.md) |
| D7 | [ESL surface syntax](d7-esl-surface-syntax.md) |
| D8 | [CompleteJson component](d8-complete-json-component.md) |
| D9 | [NbE unification and type extensions](d9-nbe-unification-and-type-extensions.md) |
| D10 | [Grothendieck institution protocol](d10-grothendieck-institution-protocol.md) |
| D11 | [Codata, streams, and resumable execution](d11-codata-streams.md) |
| D12 | [WASM extensibility](d12-wasm-extensibility.md) |
| D12b | [Orchestrator WASM plan](d12b-orchestrator-wasm-plan.md) |
| D13 | [Durable kernel state](d13-durable-kernel-state.md) |
| D14 | [Institution realisation](d14-institution-realisation.md) |
| D18 | [Ontology-as-types resolution](d18-ontology-as-types-resolution.md) |
| D19 | [Inductive types](d19-inductive-types.md) |
| D20 | [Layer reconciliation](d20-layer-reconciliation.md) |
| D21 | [Task traces and checkpointing](d21-task-traces-and-checkpointing.md) |
| D22 | [Notebook UX and TypeScript SDK](d22-notebook-and-typescript-sdk.md) |
| D23 | [Out-of-core layer architecture](d23-out-of-core-layer-architecture.md) |
| D24 | [Schema versioning](d24-schema-versioning.md) |
| D25 | [Chain consolidation](d25-chain-consolidation.md) |
| D26 | [Runtime substrate](d26-runtime-substrate.md) |
| D27 | [Julia institutions](d27-julia-institutions.md) |
| D28 | [Lean 4 as a verification institution](d28-lean-4-as-institution.md) |
| D29 | [Eigon–Julia mirror spec](d29-eigon-julia-mirror-spec.md) |
| D30 | [Eigon-to-Lean faithful translation](d30-eigon-to-lean-faithful-translation.md) |
| D31 | [External institution authoring & dispatch lifecycle](d31-external-institution-lifecycle.md) |
| D32 | [Chain-mirrored EigenTT inductives + the FormulaTerm language](d32-chain-mirrored-mini-tt-inductives.md) |
| D33 | [Partial-order chains](d33-partial-order-chains.md) |
| D34 | [Notebook chain workspace](d34-notebook-chain-workspace.md) |
| D35 | [Software-engineering knowledge graph](d35-software-engineering-knowledge-graph.md) |
| D36 | [Merge resolution UX](d36-merge-resolution-ux.md) |
| D37 | [Lambda surface and typed merge comorphisms](d37-lambda-surface-and-typed-merge-comorphisms.md) |
| D38 | [Merge provenance and witness discovery](d38-merge-provenance-and-witness-discovery.md) |
| D39 | [Justification logic](d39-justification-logic.md) |
| D40 | [Chain-mirrored Lean expressions](d40-chain-mirrored-lean-expressions.md) |
| D41 | [Commit pipeline](d41-commit-pipeline.md) |
| D42 | [Out-of-core query execution](d42-out-of-core-query-execution.md) |
| D43 | [Text and vector retrieval](d43-text-and-vector-retrieval.md) · [implementation plan](d43-implementation-plan.md) |
| D44 | [Automatic data lifecycle management](d44-automatic-data-lifecycle-management.md) |
| D45 | [BIND clause](d45-bind-clause.md) |
| D46 | [Prop universe, proof irrelevance, and axioms-as-resources](d46-prop-universe-and-proof-irrelevance.md) |
| D47 | [Chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md) |
| D48 | [Indexed inductive families](d48-indexed-inductive-families.md) |
| D49 | [ChainWitness machinery](d49-chainwitness-machinery.md) |
| D50 | [Benchmark evaluation approach](d50-benchmark-evaluation-approach.md) |
| D51 | [Benchmark implementation gaps](d51-benchmark-implementation-gaps.md) |
| D52 | [Measurement-statistics institution](d52-measurement-statistics-institution.md) |
| D53 | [Large-data tracking](d53-large-data-tracking.md) |
| D54 | [Reasoning lemma citation](d54-reasoning-lemma-citation.md) |
| D55 | [R language runtime](d55-r-language-runtime.md) |
| D56 | [Component execution and derivation materialization](d56-component-execution-and-derivation-materialization.md) |
| D57 | [schema.org vocabulary mapping](d57-schema-org-vocabulary-mapping.md) |
| D58 | [Objective framing and obligation graphs](d58-objective-framing-and-obligation-graphs.md) |
| D59 | [EigenQL array patterns and derived joins](d59-eigenql-array-patterns-and-derived-joins.md) |
| D60 | [Generic OCI tool runtime + kernel-tracked env build](d60-native-runtime-and-tracked-env-build.md) |
| D61 | [Faithful encoding of reasoning: grounding-discovery + a typed decision layer](d61-llm-based-encoding-methodology.md) |
| D62 | [The encoding pipeline: prose → typed reasoning (the driver)](d62-encoding-engine-prose-to-trees.md) |
| D63 | [The DCG engine: a categorial grammar of English over EigenTT](d63-dcg-engine-english-grammar.md) |
| D64 | [LLM-based anaphora resolution: pronouns as resolved resource references](d64-llm-anaphora-resolution.md) |
| D65 | [The lexicon runtime: lazy form-indexed lookup, per-parse scoping, lexicon identity](d65-lexicon-runtime-lazy-scoped.md) |
| D66 | [Definitional lifting: transparent definitions, explicit context, symmetric witness normalization](d66-definitional-lifting-and-witness-normalization.md) |

(Numbers D15–D17 were never assigned.)
