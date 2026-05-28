---
name: "lint"
description: "Run the canonical Eigenius formatting and lint checks."
---

# Lint

Use this skill when the user asks to run `/lint`, lint Eigenius, or verify
the workspace passes formatting and clippy before pushing.

## Commands

Both must pass cleanly:

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
```

The `-D warnings` flag promotes every clippy warning to an error. The
project does not maintain an allowlist of permitted warnings — fix the
code, do not silence the lint.

CI runs identical commands; the `just check` recipe runs both plus
`deno lint` and `deno fmt --check` for the orchestrator.

## Fixing failures

- **Format issues** (`cargo fmt --all`) — auto-applies. Run before
  committing to keep diffs minimal.
- **Clippy issues** — read the lint name; clippy's documentation explains
  the rationale and suggested fix. If a lint legitimately doesn't apply
  to a specific case, use `#[allow(clippy::<name>)]` with a one-line
  comment explaining why. Project-wide `#![allow(...)]` is not used.

## Notes

- The Deno side (`orchestration/`) has its own `deno lint` and
  `deno fmt --check` — covered by `just check`, not by this skill.
- The workspace does not use `[workspace.lints]` (Rust 1.74+); lint
  policy lives on the CI invocation.
