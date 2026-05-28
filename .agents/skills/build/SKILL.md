---
name: "build"
description: "Run the canonical Eigenius workspace build."
---

# Build

Use this skill when the user asks to run `/build`, build Eigenius, or verify
the workspace compiles.

## Command

```bash
cargo build --workspace
```

This compiles every member of the Rust workspace declared in the top-level
[`Cargo.toml`](../../../Cargo.toml). CI runs the same command.

If the build fails, fix all errors before proceeding. Warnings on their own
do not fail the build, but `/lint` treats `clippy` warnings as errors —
they will surface there.

## Notes

- The Lean toolchain pinned in `lean/runtime-worker/lean-toolchain` must be
  on `PATH` for `eigenius-lean-worker`'s `build.rs` to find `lean.h`. The
  agent engineer image bakes this in via elan.
- `protoc` is required for the kernel's `build.rs` (`tonic-build` against
  `proto/eigenius.proto`).
- Workspace-wide build does not exercise the Lean `lake` projects under
  `lean/runtime-worker/`; use `lake build` separately for those.
