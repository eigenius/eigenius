# CLAUDE.md

## Allowed commands

The following commands can be run without approval:

- `cargo build`
- `cargo test`
- `cargo clippy`
- `cargo fmt`
- `cargo check`
- `rm` (for removing old source files during refactoring)

## Project overview

Eigenius is a typed knowledge graph platform. Rust workspace with kernel, storage backends, CLI, and a Deno/TypeScript orchestration layer.

Key design docs:
- `docs/design/d1-eigon-serialization-format.md` — Eigon-JSON format spec
- `docs/design/phase0-implementation-plan.md` — detailed implementation steps
- `ontologies/core/core-ontology.json` — self-describing core ontology

## Build

```bash
source "$HOME/.cargo/env"
cargo build                    # build workspace
cargo test --workspace         # run all tests
cargo fmt --all -- --check     # formatting (must pass cleanly)
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets  # lint
```

Always run `cargo fmt --all` before committing. CI enforces formatting and will fail on unformatted code.

## Architecture

- Everything is a `Resource` — no separate Class/Property/DataType Rust types
- Core ontology is the root layer (parent=None), loaded from `core-ontology.json`
- Layers are immutable with parent pointers (`Arc<Layer>`), forming a chain
- Validator resolves definitions by walking the parent chain
- BTreeMap everywhere for deterministic ordering and cache efficiency
- Property names use snake_case, class names use PascalCase
- IRIs use the `urn:` scheme (`urn:eigenius:<namespace>:<local-name>`)
