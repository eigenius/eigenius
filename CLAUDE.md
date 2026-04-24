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

## Engineering principles

This project values long-term system health over short-term commit ease. When you discover a structural problem (wrong shape, silent corruption, inconsistent design, misaligned identity), **fix the structure** — do not paper over it with a guard, error message, or bridge. Defaulting to the smallest local fix when the underlying design is wrong creates compounding tech debt and forces future contributors to repeatedly work around the same broken foundation.

**Rule of thumb**: if the immediate fix you're considering is a *guard* against bad behavior rather than *eliminating the bad behavior*, you are about to add a Band-Aid. Stop, reconsider, and fix the structure.

**Specific signals you are wedging instead of fixing**:
- Adding a parser/runtime error to reject malformed input that should be expressible (the AST or grammar is wrong, not the input).
- Adding a "bridge" or "compatibility layer" on top of a design you've already concluded is wrong, with the intent to "clean up later." Later rarely arrives.
- Reaching for "minimal scope" or "additive change" as a justification when the foundation itself needs reshaping.
- Filing a follow-up issue immediately after writing code you already know is structurally wrong, instead of writing the code right the first time.

**When to do the proper fix in-session vs. file an issue**:
- The proper fix is in-session if: (a) the changes are still uncommitted, or (b) the structural problem actively blocks current correctness, or (c) the user is engaged and has the context. Most cases.
- File an issue only when: (a) the proper fix requires design decisions that need separate deliberation, (b) the fix is genuinely outside current scope and doesn't block forward work, and (c) the trigger is far enough out that the issue won't be closed in the same session.

This applies to AST/data-model changes, identifier schemes, lookup mechanisms, error-handling shape, public API surfaces, and ontology/resource structure. It does *not* apply to local algorithmic improvements or stylistic preferences — those are properly minimal-scope.

When in doubt, ask: "Am I solving the problem or just hiding it?" If hiding, do the harder thing.
