# Eigenius development tasks

# Build everything
build:
    cargo build --workspace

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
