# Eigenius development tasks

# Build everything
build:
    cargo build --workspace

# Build everything with CUDA support — same as `build` but the
# `eigenius-cli` binary is compiled with `--features cuda`, which
# forwards to `eigenius-embedder-candle/cuda` and lights up Candle's
# CUDA backend. Requires a CUDA 12.x toolkit on PATH (`nvcc`) and a
# compatible driver visible to the build. Runtime device choice is
# still the `[embedder].device` knob in `eigenius.toml`.
build-gpu:
    cargo build --workspace
    cargo build -p eigenius-cli --features cuda

# Same shape as `build-gpu`, but uses Candle's Metal backend instead
# of CUDA. Intended for Apple Silicon hosts.
build-metal:
    cargo build --workspace
    cargo build -p eigenius-cli --features metal

# Run all tests (Rust + Deno)
test:
    cargo test --workspace
    cd orchestration && deno test --allow-net --allow-env tests/

# Lint and format check
check:
    cargo fmt --all -- --check
    RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
    cd orchestration && deno lint && deno fmt --check

# Format all code
fmt:
    cargo fmt --all
    cd orchestration && deno fmt

# Regenerate protobuf types
generate:
    PATH="$PWD/node_modules/.bin:$PATH" buf generate
    buf lint

# Start the stack with mock LLM (no API key needed)
up-mock:
    EIGENIUS_MOCK_LLM=true docker compose up --build -d

# Start the stack. The kernel is built with `use-llm`, so its untrusted
# proposers can call a model and a D71 formalization run works on a doc branch
# with no recorded draws. Needs ANTHROPIC_API_KEY in the env.
up:
    docker compose up --build -d

# Alias for `up`, kept because it is in muscle memory and in older notes.
up-llm: up

# Start the stack with a kernel that has NO live ranker. A formalization run on
# a branch with no recorded draws fails closed here rather than parsing
# unranked; replay from a recorded `ranks.json` works fine.
up-no-llm:
    CARGO_FEATURES= docker compose up --build -d

# Copy a lexicon snapshot into the kernel's volume. `up` does NOT do this: it
# starts the stack against whatever the volume already holds, which after any
# other run is not necessarily what you think.
#
# Defaults to the NEWEST aligned snapshot rather than a pinned name — a pinned
# one goes stale at the next reseed and stages the wrong lexicon silently, which
# is the failure this recipe exists to prevent. Same `ls -1dt` autodetect
# scripts/measure-parse-rate.sh uses. Pass a path to override.
stage-snapshot snapshot="":
    #!/usr/bin/env bash
    set -euo pipefail
    root="${SNAPSHOT_ROOT:-../db-snapshot}"
    snap="{{snapshot}}"
    if [[ -z "$snap" ]]; then
        snap="$(ls -1dt "$root"/wordnet-umls-aligned-* 2>/dev/null | head -1 || true)"
        [[ -n "$snap" ]] || {
            echo "no aligned snapshot under $root (run scripts/reseed-lexicon-db.sh --umls-all," >&2
            echo "then scripts/build-alignment-snapshot.sh)" >&2
            exit 1
        }
    fi
    [[ -f "$snap/CURRENT" ]] || { echo "not a RocksDB store: $snap" >&2; exit 1; }
    snap="$(cd "$snap" && pwd)"
    # DESTRUCTIVE, and not obviously so from the name: this wipes the volume,
    # taking every branch with it — doc-<id> working branches, their glossary
    # layers, and their recorded proposer draws. Doing it under a LIVE kernel
    # also pulls RocksDB's store out from under the process. Stop it first.
    if docker compose ps --status running --services 2>/dev/null | grep -qx kernel; then
        echo "kernel is running — stopping it first (staging deletes the store it has open)"
        docker compose stop kernel >/dev/null
        restart=1
    fi
    docker run --rm -v "$snap":/src:ro -v eigenius_eigenius_db:/dst alpine \
        sh -c 'rm -rf /dst/* && cp -a /src/. /dst/'
    echo "staged $(basename "$snap") into eigenius_eigenius_db (all previous branches are gone)"
    [[ -n "${restart:-}" ]] && { docker compose start kernel >/dev/null; echo "kernel restarted"; }
    true

# Which snapshot is currently staged (the reseed stamps a PROVENANCE file).
which-snapshot:
    @docker run --rm -v eigenius_eigenius_db:/db alpine \
        sh -c 'cat /db/PROVENANCE 2>/dev/null || echo "no PROVENANCE — volume was not staged from a snapshot"'

# Stop the stack
down:
    docker compose down

# Run the end-to-end demo
demo:
    ./demo/run.sh

# Run the IntervalArithmetic institution end-to-end demo (D31)
demo-intervals:
    ./demo/intervals/run.sh

# Run the Symbolics institution end-to-end demo (D27 §4.1 / Phase 19d)
demo-symbolics:
    ./demo/symbolics/run.sh

# Run the Catalyst institution end-to-end demo (D27 §4.4 / Phase 19h)
demo-catalyst:
    ./demo/catalyst/run.sh

# Run the DiffEq institution end-to-end demo (D27 §4.5 / Phase 19g)
demo-diffeq:
    ./demo/diffeq/run.sh

# Run the JuMP-HiGHS institution end-to-end demo (D27 §4.2 / Phase 19f)
demo-jump-highs:
    ./demo/jump-highs/run.sh

# Start orchestrator locally (mock LLM)
orchestrator-mock:
    cd orchestration && EIGENIUS_MOCK_LLM=true deno run --allow-net --allow-env --allow-sys=hostname src/main.ts

# Start orchestrator locally (real LLM)
orchestrator:
    cd orchestration && deno run --allow-net --allow-env --allow-sys=hostname src/main.ts

# Start kernel locally with orchestrator
serve:
    cargo run -p eigenius-cli -- serve --orchestrator http://localhost:8080

# Compile an ESL file to Eigon-JSON
compile file:
    cargo run -q -p eigenius-cli -- compile {{file}}

# Load a file into the local kernel
load file:
    cargo run -q -p eigenius-cli -- load {{file}}

# Validate a file against the ontology
validate file:
    cargo run -q -p eigenius-cli -- validate {{file}}
