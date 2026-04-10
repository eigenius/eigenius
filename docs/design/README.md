# Design Documents

This directory contains the architecture specification, implementation plan, and design documents for Eigenius.

## Core Documents

- **[architecture-v0.3.md](architecture-v0.3.md)** — The current architecture specification. The authoritative reference for all design decisions.
- **[implementation-plan.md](implementation-plan.md)** — Six-phase build plan with deliverables, test plans, and design document requirements.
- **[architecture-v0.2-review.md](architecture-v0.2-review.md)** — Design review of the v0.2 architecture, identifying contradictions and gaps (most resolved in v0.3).

## Design Documents (to be written)

Per the implementation plan, the following design documents are required:

| # | Document | Required before |
|---|----------|-----------------|
| D1 | Eigon Serialization Format | Phase 0 |
| D2 | EigenQL v1 Concrete Syntax | Phase 1 |
| D3 | DAG Specification Format | Phase 2 |
| D4 | TiKV Key Encoding & Deployment | Phase 3 |
| D5 | gRPC API Specification | Phase 3 |
| D6 | Reasoning Trace Schema | Phase 4 |
| D7 | Capability SDK & WASM Interface | Phase 5 |
| D8 | Capability Protocol Wire Format | Phase 5 |
| D9 | Security Model | Phase 3+ |
| D10 | Ontology Versioning & Evolution | Phase 3+ |
| D11 | Execution Context Internals | Phase 2 |
| D12 | Observability & Operational Tooling | Phase 4 |
| D13 | Capability Versioning | Phase 5 |

Each document should be added to this directory as `D{N}-{short-name}.md`.
