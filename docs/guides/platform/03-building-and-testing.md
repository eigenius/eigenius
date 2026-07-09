# 3. Building and testing

The development workflow is driven by [`just`](https://github.com/casey/just). The recipes are short — every one is a thin wrapper over `cargo`, `deno`, or a shell loop. Knowing the recipes is equivalent to knowing the workflow.

The full recipe list is in [`justfile`](../../../justfile).

## 3.1. The four core recipes

```bash
just build      # workspace build
just test       # cargo test --workspace + deno test
just check      # cargo fmt --check + clippy + deno lint + deno fmt --check
just fmt        # cargo fmt --all + deno fmt
```

These four cover everything in the day-to-day cycle.

## 3.2. `just build` in detail

```bash
just build      # equivalent to: cargo build --workspace
```

This compiles the full Rust workspace (kernel, storage backends, CLI, and
the institution crates). No extra toolchain is required beyond stable Rust.

## 3.3. `just test` in detail

```bash
just test       # cargo test --workspace
                # cd orchestration && deno test --allow-net --allow-env tests/
```

Two test pools:

- **Rust tests** (`cargo test --workspace`) — every crate's unit and integration tests. The kernel tests cover ontology validation, layer chain resolution, EigenQL parsing/evaluation, ESL compilation, NbE type checking, institution dispatch, etc. Heavy; the workspace has thousands of tests across roughly 200 modules.
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

## 3.6. GPU acceleration for vector embeddings (optional)

D43 vector retrieval ships [BGE-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5) via [HuggingFace Candle](https://github.com/huggingface/candle), wired into the `eigenius serve` binary so the embedder pool registers at startup and the post-Load sweep fires automatically against any layer that declares a `core:VectorIndex` Resource. The default build is CPU-only and requires no extra toolchain. To opt into GPU inference:

```bash
just build-gpu      # CUDA — workspace + CLI with --features cuda
just build-metal    # Apple Silicon equivalent
cargo build-gpu     # alias for `cargo build -p eigenius-cli --features cuda`
```

`build-gpu` forwards `--features cuda` to `eigenius-embedder-candle`, which lights up Candle's CUDA backend (cuBLAS / cuDNN). The CPU-default workspace build is still produced first so unrelated crates don't grow a transitive CUDA dependency; only the CLI binary picks up the feature.

Requirements on the host:

- **CUDA build**: CUDA 12.x toolkit on `PATH` (`nvcc`), `libclang-dev` (for `bindgen`), and a matching NVIDIA driver visible to the build.
- **Metal build**: macOS on Apple Silicon. No extra toolchain.

The runtime device choice is independent of the build flag — set it in `eigenius.toml`:

```toml
[embedder]
enabled = ["bge-small-en-v1.5"]
device = "auto"      # auto | cpu | cuda | metal
batch_size = 32
fail_fast_on_missing_model = true
```

Or via env (`EIGENIUS_EMBEDDER_ENABLED`, `EIGENIUS_EMBEDDER_DEVICE`, `EIGENIUS_EMBEDDER_BATCH_SIZE`, `EIGENIUS_EMBEDDER_FAIL_FAST_ON_MISSING_MODEL`). Resolution order is defaults → file → env → construction overrides; the schema lives in [`crates/eigenius-config/src/embedder.rs`](../../../crates/eigenius-config/src/embedder.rs). `device = "auto"` (default) picks the accelerator the binary was compiled with and falls back to CPU on init failure; `device = "cpu"` forces CPU even on a CUDA build. The `select_device()` helper inside the embedder is what enforces this — see [`crates/eigenius-embedder-candle/src/lib.rs`](../../../crates/eigenius-embedder-candle/src/lib.rs).

For deploying the GPU build in Docker (kernel image + `docker-compose.gpu.yml` override), see [chapter 5 — Running locally](05-running-locally.md).

Measured speedup, 1 007 GO Class corpus, batch=32, RTX 4070 (see [docs/notes/d43-implementation-notes.md](../../notes/d43-implementation-notes.md) for the full table + caveats):

| device | sweep | per-query |
|---|---|---|
| CPU, per-text | 162 s | ~130 ms |
| **CUDA, batched** | **3.62 s** | **~30 ms** |

## 3.7. Other recipes

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

## 3.8. Build artifact locations

| Artifact | Location |
|---|---|
| Workspace binaries | `target/debug/` (and `target/release/` for `--release`) |
| `eigenius` CLI binary | `target/debug/eigenius` |
| Deno-cached deps | `~/.cache/deno/` |
| Docker images | local Docker daemon (`docker images | grep eigenius`) |

## 3.9. Common build issues

The frequent culprits, in rough order of frequency:

- **`error: failed to run custom build command for prost-build`** — `protobuf-compiler` not installed. Install it (chapter 2).
- **`error: linker 'cc' not found`** — `build-essential` missing on Ubuntu/WSL. Install it.
- **`error[E0658]: ...`** referencing a Rust version — your `rustc` is older than 1.97. Run `rustup update`.
- **`error: failed to run custom build command for librocksdb-sys`** — `libclang-dev` missing. Install it.
- **Deno cache stale** — `deno cache --reload orchestration/src/main.ts`.

For ongoing build issues, [chapter 13](13-troubleshooting.md) collects them by symptom.

---

Next: **[4. CLI reference →](04-cli-reference.md)**
