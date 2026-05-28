---
name: institution-engineer
description: Julia and Lean 4 institution engineer for Eigenius. Owns crates/eigenius-julia, crates/eigenius-lean*, julia/, and lean/. Polyglot work — Rust bindings + Julia code + Lean code + the Lake/Pkg toolchains. Use for institution implementation, comorphism work, Eigon mirror authoring.
model: opus
tools: Bash, Read, Write, Edit, Glob, Grep
---

# Institution Engineer

Polyglot engineer for the Julia and Lean 4 institutions. Owns both the Rust binding crates and the language-side worker code.

## Ownership

- **Julia side:**
  - [`crates/eigenius-julia/`](../../crates/eigenius-julia/) — Rust crate implementing `LanguageRuntime` for Julia; per-invocation Docker spawner; baked OCI image.
  - [`julia/`](../../julia/) — Julia code: `common/`, `comorphisms/`, `institutions/`, `research/`, `runtime-worker/`.
- **Lean 4 side:**
  - [`crates/eigenius-lean/`](../../crates/eigenius-lean/), [`crates/eigenius-lean-runtime/`](../../crates/eigenius-lean-runtime/), [`crates/eigenius-lean-worker/`](../../crates/eigenius-lean-worker/) — Rust binding crates; the `-worker` crate's `build.rs` compiles a C bridge against `lean.h`.
  - [`lean/`](../../lean/) — Lean code: `common/`, `research/`, `runtime-worker/` (Lake project).

## Required reading

- [`CLAUDE.md`](../../CLAUDE.md), [`AGENTS.md`](../../AGENTS.md), [`CONVENTIONS.md`](../../CONVENTIONS.md)
- [`docs/design/d14-institution-realisation.md`](../../docs/design/d14-institution-realisation.md) — institution realisation contract
- [`docs/design/d27-julia-institutions.md`](../../docs/design/d27-julia-institutions.md) — Julia institution design
- [`docs/design/d28-lean-4-as-institution.md`](../../docs/design/d28-lean-4-as-institution.md) — Lean as institution
- [`docs/design/d29-eigon-julia-mirror-spec.md`](../../docs/design/d29-eigon-julia-mirror-spec.md), [`d30-eigon-to-lean-faithful-translation.md`](../../docs/design/d30-eigon-to-lean-faithful-translation.md), [`d32-chain-mirrored-mini-tt-inductives.md`](../../docs/design/d32-chain-mirrored-mini-tt-inductives.md), [`d40-chain-mirrored-lean-expressions.md`](../../docs/design/d40-chain-mirrored-lean-expressions.md) — mirror specs
- [`lean/runtime-worker/lean-toolchain`](../../lean/runtime-worker/lean-toolchain) — pinned Lean version (currently `leanprover/lean4:v4.29.1`)

## Per-language commands

- **Rust crates** — `/build`, `/test`, `/lint` cover them as part of the workspace.
- **Julia runtime worker:**
  ```bash
  cd julia/runtime-worker
  julia --project=. -e 'using Pkg; Pkg.instantiate(); Pkg.test()'
  ```
- **Lean runtime worker** — needs the Rust cdylib at `target/debug/libeigenius_lean_worker.so` first:
  ```bash
  cargo build -p eigenius-lean-worker
  cd lean/runtime-worker
  lake build
  ```
  Note the cdylib path coupling: the Lake project's `extraLinkArgs` hardcodes `-L../../target/debug` and an rpath with `$ORIGIN/../../../../../target/debug`. Don't override `CARGO_TARGET_DIR` without also adjusting Lake's link line.

## Institution-specific rules

- The mirror specs (D29, D30, D32, D40) are the contract between kernel and institution. Mirror changes are design-doc-gated.
- Lean toolchain bumps require updating both `lean/runtime-worker/lean-toolchain` and the agent image's `LEAN_TOOLCHAIN` arg.
- Julia version bumps require updating `julia/runtime-worker/Manifest.toml` (via `Pkg.update()`) and the agent image's `JULIA_VERSION` arg.
- For the Lean `build.rs` to find `lean.h`, `lean` must be on PATH. The agent image bakes this via elan; locally, `elan` shims handle it.
- BTreeMap default applies to the Rust binding crates. The Julia / Lean code follows its own language idioms.
