# Eigenius development tasks

# Build everything
build: build-wasm
    cargo build --workspace
    cargo build --manifest-path orchestration/native/Cargo.toml

# Build WASM examples and copy fixtures for tests
build-wasm:
    cd examples/wasm-cbor-echo && cargo component build
    cd examples/wasm-doc-validator && cargo component build
    cd examples/wasm-http-shout && cargo component build
    cd examples/wasm-read-query-probe && cargo component build
    cd examples/wasm-d14-echo && cargo component build
    cd examples/wasm-d14-dock && cargo component build
    cd examples/wasm-d14-assay && cargo component build
    cd examples/wasm-d14-arrhenius && cargo component build
    mkdir -p kernel/tests/fixtures
    cp examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm kernel/tests/fixtures/
    cp examples/wasm-d14-echo/target/wasm32-unknown-unknown/debug/eigenius_wasm_d14_echo.wasm kernel/tests/fixtures/
    cp examples/wasm-d14-dock/target/wasm32-unknown-unknown/debug/eigenius_wasm_d14_dock.wasm kernel/tests/fixtures/
    cp examples/wasm-d14-assay/target/wasm32-unknown-unknown/debug/eigenius_wasm_d14_assay.wasm kernel/tests/fixtures/
    cp examples/wasm-d14-arrhenius/target/wasm32-unknown-unknown/debug/eigenius_wasm_d14_arrhenius.wasm kernel/tests/fixtures/

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
