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

# Start the stack with real LLM
up:
    docker compose up --build -d

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
