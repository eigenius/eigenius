# 2. Installation and prerequisites

The platform builds and runs on Linux (native or Windows with WSL 2) and macOS. The required toolchain is a Rust kernel, a Deno orchestrator, and a CLI tied together by gRPC. Optional pieces (Docker deployment, GitHub workflow) add their own tools.

## 2.1. Required toolchain

### Rust 1.97+

The Rust version pinned by [`deploy/Dockerfile.kernel`](../../../deploy/Dockerfile.kernel) is **1.97**. Earlier versions may fail to build some workspace dependencies.

Install via [rustup](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

Verify:

```bash
rustc --version  # rustc 1.97.0 or newer
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

Without `just`, every recipe in [`justfile`](../../../justfile) can be run manually as plain shell — `just build` is `cargo build --workspace`, `just test` is `cargo test --workspace` plus `deno test`, and so on.

## 2.4. Optional: Docker

The end-to-end demo can run entirely in containers — skips Rust and Deno on the host. Install Docker Engine and Compose v2 per your distribution's instructions, then see [chapter 12](12-deployment.md).

## 2.5. Optional: GitHub `gh` CLI

Project tracking happens in GitHub Issues. The `gh` CLI is the smoothest way to read and file them:

```bash
sudo apt-get install -y gh   # Ubuntu / WSL 2
brew install gh              # macOS
gh auth login
```

## 2.6. Optional: domain corpora (lexicon / knowledge-graph sources)

Three third-party corpora can be imported as typed layers: **WordNet** (the general
lexicon behind the DCG engine), **NCBI Gene**, and **UMLS** (domain knowledge-graph
sources, D65 §5). None is vendored in this repo (`references/` is gitignored) — each is
provisioned on demand by a script that does download/extract → convert → load. You only
need these if you are working on the lexicon / DCG engine or the domain knowledge graph.

Each importer is **deterministic** (no LLM); `--validate` (compile + felicity-gate = an
in-memory load proof) always runs, and `--endpoint 127.0.0.1:50051` additionally commits
the layer into a running `eigenius serve --db <path>`, persisted like any other layer
(see [§6 Database management](06-database-management.md)). Emitted `.esl` documents are
gitignored — they are regenerable build artifacts and carry their source's license
notice at the head.

### 2.6.1. WordNet (the DCG / natural-language engine)

The English grammar engine (D63) parses prose against a lexicon imported from **WordNet
3.0**.

```bash
scripts/provision-wordnet.sh                            # full import: download + convert + validate
scripts/provision-wordnet.sh --seed gene --seed depend  # a small SEEDED slice (fast — for trying it out)
scripts/provision-wordnet.sh --endpoint 127.0.0.1:50051 # ... and persist into a running `eigenius serve`
```

- **download** — fetches WordNet 3.0 into `references/WordNet-3.0/` (idempotent). Override the source with `WORDNET_URL=<mirror>`, optionally verify with `WORDNET_SHA256=<digest>`.
- **convert** — `wordnet-import` → an Eigon-ESL lexicon. The full import is ~325k `LexicalEntry` resources (a few minutes to validate); a `--seed`/`--limit` slice is fast. Output `wordnet-full.esl` (~150 MB, gitignored).

The emitted lexicon embeds WordNet content (glosses, lemmas, the synset lattice), so it is a derivative work and carries the **WordNet 3.0 license notice** (Princeton's license permits redistribution with that notice).

### 2.6.2. NCBI Gene

A typed mirror (`ncbi:Gene` witnesses) plus a derived lexicon (`lexicon:ncbi_gene`),
imported from NCBI Gene's `gene_info`. NCBI data is a U.S. Government public-domain work.

```bash
scripts/provision-ncbi-gene.sh                          # download + convert + validate (Homo sapiens)
scripts/provision-ncbi-gene.sh --wordnet-anchor         # also emit ncbi:Gene ⊑ wn:gene.n.01 (needs WordNet on the chain)
scripts/provision-ncbi-gene.sh --endpoint 127.0.0.1:50051   # ... + load into a service
```

- **download** — fetches `gene_info` into `references/ncbi/` (idempotent). Override the organism with `TAX_ID=<id>` (default `9606` = human) and the source with `GENE_INFO_URL=<url>`.
- **convert** — `ncbi-gene-import` → `ncbi-gene.esl` (gitignored). `--wordnet-anchor` only validates on a chain that already has WordNet.

### 2.6.3. UMLS

A typed mirror (`umls:Concept` classes under `umls:SemanticType` classes) plus a derived
lexicon (`lexicon:umls`), imported from the UMLS Metathesaurus.

> **UMLS is licensed, not public-domain.** You must hold your own UMLS Metathesaurus
> License and download the release yourself — the script does **not** fetch it. Place the
> Level-0 Metathesaurus zip at `references/umls-<release>-metathesaurus-level0.zip` (e.g.
> `references/umls-2026AA-metathesaurus-level0.zip`), then run the script. Only **SRL-0
> (Level 0)** sources are emitted, and the output carries the UMLS license notice plus the
> redistribution constraint.

```bash
scripts/provision-umls.sh                          # WRN-relevant semantic-type subset (default)
scripts/provision-umls.sh --all                    # ALL semantic types (large — the ~281k-resource chain)
scripts/provision-umls.sh --tui T047 --tui T028    # custom semantic-type (TUI) allowlist
scripts/provision-umls.sh --endpoint 127.0.0.1:50051
```

- **extract** — unzips only the RRF files the importer needs (MRCONSO/MRSTY/MRSAB/MRRANK/MRDEF) into `references/umls/<release>/META/`. Override with `UMLS_ZIP=<path>` and `RELEASE=<label>` (default `2026AA`).
- **convert** — `umls-import` → `umls.esl` (gitignored). Default keeps a WRN-paper-relevant semantic-type allowlist; `--all` imports everything (large).

## 2.7. WSL 2 notes

All toolchain installs land in the WSL distribution (Ubuntu, etc.), not in Windows itself. Expected practice:

- Edit code from Windows using VS Code's WSL remote extension.
- Build, test, and run inside WSL.
- The kernel (port 50051) and orchestrator (port 8080) are reachable from Windows at `localhost:<port>` thanks to WSL 2's networking integration.

If `cargo build` is slow, the WSL filesystem may be using `/mnt/c/...` (the Windows filesystem mounted into WSL); cloning the repo into the native WSL filesystem (`~/src/eigenius`) is dramatically faster for build operations.

## 2.8. Verifying the install

After all required tools are installed:

```bash
# from the repo root
just build       # cargo build --workspace
# or
cargo build --workspace
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

## 2.9. What to install next

| If you want to … | Install |
|---|---|
| Run CLI commands against in-process file ops | Just the Rust toolchain |
| Run the demo end-to-end | Add Deno (and optionally Docker) |
| Run the kernel test suite | Just the Rust toolchain |
| Deploy to Docker Compose or Azure | Add Docker (locally) and `az` CLI (for Azure) |

---

Next: **[3. Building and testing →](03-building-and-testing.md)**
