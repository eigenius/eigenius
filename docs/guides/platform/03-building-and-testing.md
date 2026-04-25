# 3. Building and testing

The development workflow is driven by [`just`](https://github.com/casey/just). The recipes are short — every one is a thin wrapper over `cargo`, `deno`, or a shell loop. Knowing the recipes is equivalent to knowing the workflow.

The full recipe list is in [`justfile`](../../../justfile).

## 3.1. The four core recipes

```bash
just build      # workspace build + WASM examples + test fixture copy
just test       # cargo test --workspace + deno test
just check      # cargo fmt --check + clippy + deno lint + deno fmt --check
just fmt        # cargo fmt --all + deno fmt
```

These four cover everything in the day-to-day cycle.

## 3.2. `just build` in detail

```bash
just build      # equivalent to: just build-wasm && cargo build --workspace
                #                                && cargo build --manifest-path orchestration/native/Cargo.toml
```

The `build-wasm` dependency does two things:

1. **Compile every `examples/wasm-*` crate** with `cargo component build` (one per directory).
2. **Copy two of the resulting `.wasm` binaries into [`kernel/tests/fixtures/`](../../../kernel/tests/fixtures/)** — `eigenius_wasm_doc_validator.wasm` and `eigenius_wasm_ordering_institution.wasm`. The kernel test suite loads these via `include_bytes!` to verify end-to-end WASM dispatch.

If you don't have the WASM toolchain installed, `just build-wasm` fails. You can still run `cargo build --workspace` directly to build the rest of the platform — but the WASM-fixture-dependent kernel tests will fail at compile time.

The orchestration `native/` build is a small Rust adapter linked into the orchestrator's Deno runtime via FFI; it builds quickly and rarely changes.

## 3.3. `just test` in detail

```bash
just test       # cargo test --workspace
                # cd orchestration && deno test --allow-net --allow-env tests/
```

Two test pools:

- **Rust tests** (`cargo test --workspace`) — every crate's unit and integration tests. The kernel tests cover ontology validation, layer chain resolution, EigenQL parsing/evaluation, ESL compilation, NbE type checking, WASM capability hosting, etc. Heavy; the workspace has thousands of tests across roughly 200 modules.
- **Deno tests** (`deno test`) — orchestrator-side tests covering the LLM adapter, component dispatch, and MCP server.

Both must pass cleanly before merging.

To run a single Rust test by name:

```bash
cargo test -p eigenius-kernel test_name_pattern
```

To run a single Deno test:

```bash
cd orchestration
deno test --allow-net --allow-env tests/some-test.ts
```

## 3.4. `just check` in detail

```bash
just check      # cargo fmt --all -- --check
                # RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
                # cd orchestration && deno lint && deno fmt --check
```

CI runs the equivalent of `just check` plus `just test`. A green local `just check && just test` is the baseline before opening a PR.

`RUSTFLAGS="-D warnings"` upgrades clippy warnings to errors — the project enforces a zero-warnings policy in CI.

## 3.5. WASM-only build

If you're iterating on a single WASM example:

```bash
just build-wasm                      # all examples + test fixture copy
cd examples/wasm-doc-validator       # specific example
cargo component build
```

The output lives at `target/wasm32-unknown-unknown/debug/<crate_name>.wasm`.

## 3.6. Other recipes

```bash
just generate           # regenerate protobuf types (requires `buf`)
just up                 # docker compose up --build -d (real LLM)
just up-mock            # docker compose up --build -d (mock LLM)
just down               # docker compose down
just demo               # ./demo/run.sh
just orchestrator       # run orchestrator locally (real LLM)
just orchestrator-mock  # run orchestrator locally (mock LLM)
just serve              # cargo run -p eigenius-cli -- serve --orchestrator http://localhost:8080
just compile <file>     # eigenius compile <file>
just load <file>        # eigenius load <file>
just validate <file>    # eigenius validate <file>
```

The Docker recipes (`up`, `up-mock`, `down`) are the easiest way to get the full stack running without three terminals — see [chapter 5](05-running-locally.md).

The single-command recipes (`compile`, `load`, `validate`) are convenience shortcuts for the most common ad-hoc commands; for everything else, drop to the `eigenius` CLI directly.

## 3.7. Build artifact locations

| Artifact | Location |
|---|---|
| Workspace binaries | `target/debug/` (and `target/release/` for `--release`) |
| `eigenius` CLI binary | `target/debug/eigenius` |
| WASM example binaries | `examples/wasm-*/target/wasm32-unknown-unknown/debug/*.wasm` |
| Test fixtures (copied) | `kernel/tests/fixtures/*.wasm` |
| Deno-cached deps | `~/.cache/deno/` |
| Docker images | local Docker daemon (`docker images | grep eigenius`) |

## 3.8. Common build issues

The frequent culprits, in rough order of frequency:

- **`error: failed to run custom build command for prost-build`** — `protobuf-compiler` not installed. Install it (chapter 2).
- **`error: linker 'cc' not found`** — `build-essential` missing on Ubuntu/WSL. Install it.
- **`error[E0658]: ...`** referencing a Rust version — your `rustc` is older than 1.95. Run `rustup update`.
- **`error: failed to run custom build command for librocksdb-sys`** — `libclang-dev` missing. Install it.
- **`cargo component: command not found`** — needed for WASM examples. `cargo install cargo-component`.
- **WASM target not installed** — `rustup target add wasm32-unknown-unknown`.
- **Deno cache stale** — `deno cache --reload orchestration/src/main.ts`.

For ongoing build issues, [chapter 12](12-troubleshooting.md) collects them by symptom.

---

Next: **[4. CLI reference →](04-cli-reference.md)**
