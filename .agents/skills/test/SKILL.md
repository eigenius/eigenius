---
name: "test"
description: "Run the canonical Eigenius workspace test suite."
---

# Test

Use this skill when the user asks to run `/test`, test Eigenius, or verify
behaviour before pushing.

## Command

```bash
cargo test --workspace
```

Runs every workspace member's unit tests and integration tests. CI runs the
same invocation. Tests are a mix of inline `#[cfg(test)] mod tests` blocks
and separate `tests/` directories; both are picked up automatically.

All tests must pass before requesting review. If a test fails, investigate
and fix — do not skip or comment out.

## Notes

- `proptest` is used in several kernel modules; failures show a minimised
  counterexample. Capture the seed (`CARGO_PROPTEST_SEED=...`) if you need
  to reproduce locally.
- Integration tests in `tests/` directories build their own test binaries —
  expect the test run to recompile after kernel-internal changes that ripple
  through public types.
- Tests for the Lean and Julia institutions exercise the Rust crate side
  only; full institution behaviour (Lean `lake test`, Julia `Pkg.test`) is
  separate from `cargo test --workspace`.
