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

## Working protocol (default for substantive tasks)

For any non-trivial task with a **claim worth defending** (analysis, reproduction,
design decisions, debugging conclusions, research), the default method is the
**`reasoning`** skill (`.claude/skills/reasoning.md`): capture reasoning live as a
typed Eigenius chain — every load-bearing claim is a graded, witnessed proposition
the kernel commit gate accepts (`Holds`) or rejects (`Fails`). Read that skill at
task start; use the `eigenius` skill for the platform mechanics.

The core disciplines, always in force (even before reaching for the chain):
- **Don't assert — witness.** Empirical claims are *Derived* (a run/test/proof
  produced the result) or they are an explicitly-graded *Declared* hypothesis —
  never stated as established fact unwitnessed.
- **Check before you conclude.** Produce the evidence *before* the claim, not after.
- **Fail closed.** An unsupported result is a stop-and-investigate that gets
  *recorded as a finding* — never silently dropped, weakened, or routed around.
- **Anchor new territory** in real, cited prior knowledge (verified DOIs/PMIDs;
  never fabricate a source); distinguish anchors and assumptions from established
  fact. The `grounding` skill is the method: retrieve from the kernel first (D43),
  research the gap, map results back in as anchors + aligned standard vocabulary.
- **Plan deviations are structural, not silent** — surface and justify any
  departure from an agreed plan rather than quietly changing course.
- **Grade every claim** Observed / Declared / Derived / Verified.

Trivial mechanical work (renames, formatting, single-fact lookups) has no thesis
and is exempt. When unsure: if being wrong about a claim would be embarrassing, it
gets a witness.

## Build

```bash
cargo build                    # build workspace
cargo test --workspace         # run all tests
cargo fmt --all -- --check     # formatting (must pass cleanly)
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets  # lint
```

Cargo is already on `PATH` — do **not** prefix commands with `source "$HOME/.cargo/env" && …`. The compound form gets matched as a single literal by Claude Code's permission system and won't hit the `Bash(cargo …*)` allow rules, so it pointlessly prompts.

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

**Project posture.** Eigenius is the foundation of a platform intended to change how typed knowledge is represented, processed, and verified. There is no tight release timeline. **Getting the design right matters more than getting it done quickly.** Time pressure is not a valid reason to compromise structural decisions — there is no time pressure. If a proper fix is multi-session, multi-day, or multi-week, that is fine; invest the time. The cost of shipping a wrong shape is paid continuously by everyone who works on the system afterward, and paid most heavily when the wrong shape has to be unwound under the pressure of downstream consumers. Avoid that entirely by building the right shape the first time.

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
