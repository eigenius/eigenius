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
    cd examples/wasm-ordering-institution && cargo component build
    cd examples/wasm-read-query-probe && cargo component build
    mkdir -p kernel/tests/fixtures
    cp examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm kernel/tests/fixtures/
    cp examples/wasm-ordering-institution/target/wasm32-unknown-unknown/debug/eigenius_wasm_ordering_institution.wasm kernel/tests/fixtures/

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
