# 2. Installation and prerequisites

The platform builds and runs on Linux (native or Windows with WSL 2) and macOS. The required toolchain is a Rust kernel, a Deno orchestrator, and a CLI tied together by gRPC. Optional pieces (WASM examples, Docker deployment, GitHub workflow) add their own tools.

## 2.1. Required toolchain

### Rust 1.95+

The Rust version pinned by [`deploy/Dockerfile.kernel`](../../../deploy/Dockerfile.kernel) is **1.95**. Earlier versions fail to build `wasmtime 43`, on which the WASM runtime depends.

Install via [rustup](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

Verify:

```bash
rustc --version  # rustc 1.95.0 or newer
```

### Deno

Used by the orchestrator. Install via the official one-liner:

```bash
curl -fsSL https://deno.land/install.sh | sh
```

Or via Homebrew on macOS: `brew install deno`.

### System packages (Ubuntu / WSL 2)

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev protobuf-compiler libclang-dev
```

What each is for:

- `build-essential` — C/C++ toolchain for RocksDB's native sources.
- `pkg-config` + `libssl-dev` — TiKV client (kept around even though TiKV backend is a placeholder, so the workspace builds).
- `protobuf-compiler` — `protoc` for the gRPC build scripts.
- `libclang-dev` — bindgen needs it to compile RocksDB headers.

On macOS, the equivalent comes from Xcode Command Line Tools plus Homebrew (`brew install protobuf llvm`).

## 2.2. Recommended: `just`

The repository uses [`just`](https://github.com/casey/just) as a task runner. The commands documented in this guide assume `just` is available.

```bash
cargo install just
```

Without `just`, every recipe in [`justfile`](../../../justfile) can be run manually as plain shell — `just build` is `cargo build --workspace` plus a WASM step, `just test` is `cargo test --workspace` plus `deno test`, and so on.

## 2.3. Optional: WASM toolchain

Required if you intend to build WASM components or institutions, or to run the full kernel test suite (some kernel tests load WASM fixtures via `include_bytes!`).

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-component
```

After installation, `just build` (or `just build-wasm`) builds every WASM example under [`examples/wasm-*`](../../../examples/) and copies the binaries into [`kernel/tests/fixtures/`](../../../kernel/tests/fixtures/) so the kernel tests can find them.

## 2.4. Optional: Docker

The end-to-end demo can run entirely in containers — skips Rust and Deno on the host. Install Docker Engine and Compose v2 per your distribution's instructions, then see [chapter 11](11-deployment.md).

## 2.5. Optional: GitHub `gh` CLI

Project tracking happens in GitHub Issues. The `gh` CLI is the smoothest way to read and file them:

```bash
sudo apt-get install -y gh   # Ubuntu / WSL 2
brew install gh              # macOS
gh auth login
```

## 2.6. WSL 2 notes

All toolchain installs land in the WSL distribution (Ubuntu, etc.), not in Windows itself. Expected practice:

- Edit code from Windows using VS Code's WSL remote extension.
- Build, test, and run inside WSL.
- The kernel (port 50051) and orchestrator (port 8080) are reachable from Windows at `localhost:<port>` thanks to WSL 2's networking integration.

If `cargo build` is slow, the WSL filesystem may be using `/mnt/c/...` (the Windows filesystem mounted into WSL); cloning the repo into the native WSL filesystem (`~/src/eigenius`) is dramatically faster for build operations.

## 2.7. Verifying the install

After all required tools are installed:

```bash
# from the repo root
just build       # full build + WASM examples (with WASM toolchain)
# or
cargo build --workspace   # workspace build only (no WASM examples)
```

A successful `cargo build --workspace` exits cleanly with the binaries under `target/debug/`:

- `target/debug/eigenius` — the CLI binary
- Other crate libraries (kernel, storage, etc.)

Then verify the orchestrator:

```bash
cd orchestration
deno cache src/main.ts
```

A clean `deno cache` resolves all TypeScript dependencies without errors.

## 2.8. What to install next

| If you want to … | Install |
|---|---|
| Run CLI commands against in-process file ops | Just the Rust toolchain |
| Run the demo end-to-end | Add Deno (and optionally Docker) |
| Run the kernel test suite | Add WASM toolchain (`wasm32-unknown-unknown`, `cargo-component`) |
| Build your own WASM extensions | WASM toolchain, plus `eigenius-wasm-sdk` Cargo dep in your project |
| Deploy to Docker Compose or Azure | Add Docker (locally) and `az` CLI (for Azure) |

---

Next: **[3. Building and testing →](03-building-and-testing.md)**
