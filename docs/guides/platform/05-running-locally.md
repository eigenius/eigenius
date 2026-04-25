# 5. Running the platform locally

Two ways to bring the platform up locally: the **three-terminal model** (one each for orchestrator, kernel, CLI/demo) and **Docker Compose** (a single `up` command). The three-terminal model is easier for debugging the orchestrator or kernel directly; Docker Compose is the fast path for running demos.

## 5.1. Three-terminal model

### Terminal 1 — orchestrator

The orchestrator listens on port 8080. With a real LLM:

```bash
cd orchestration
ANTHROPIC_API_KEY=sk-ant-... deno run --allow-net --allow-env src/main.ts
```

Or using the `just` recipe:

```bash
just orchestrator
```

Without an API key, use mock mode:

```bash
EIGENIUS_MOCK_LLM=true just orchestrator-mock
```

Mock mode returns canned responses for `CompleteText` and `CompleteJson` calls — useful for smoke tests and CI.

### Terminal 2 — kernel

The kernel listens on port 50051. To talk to the orchestrator you started in terminal 1, point it at `http://localhost:8080`:

```bash
cargo run -p eigenius-cli -- serve --orchestrator http://localhost:8080
```

Or:

```bash
just serve
```

For persistent state, add `--db <path>`:

```bash
cargo run -p eigenius-cli -- serve --db /tmp/eigenius.db --orchestrator http://localhost:8080
```

The kernel logs each gRPC call as it arrives. Watch for `INFO eigenius_kernel::server` lines indicating successful startup.

### Terminal 3 — CLI / demo

Once both services are up, drive the system from the CLI:

```bash
# Inspect a core resource
eigenius --endpoint http://localhost:50051 inspect "urn:eigenius:core:Class"

# Load a file
eigenius --endpoint http://localhost:50051 load demo/document.json

# Run a program
eigenius --endpoint http://localhost:50051 run demo/summarize.esl demo/input.json

# Or run the full demo
./demo/run.sh
```

## 5.2. Docker Compose

Brings up both services with one command. Configuration in [`docker-compose.yml`](../../../docker-compose.yml).

### Starting

```bash
# Mock LLM (no API key needed)
EIGENIUS_MOCK_LLM=true docker compose up --build -d
# or via just:
just up-mock

# Real LLM
ANTHROPIC_API_KEY=sk-ant-... docker compose up --build -d
# or:
just up
```

The first `--build` invocation can take 5–10 minutes (full Rust build inside the container). Subsequent starts use cached layers and finish in seconds.

### Verifying

```bash
# Orchestrator health
curl http://localhost:8080/health

# Kernel reachable
eigenius --endpoint http://localhost:50051 inspect "urn:eigenius:core:Class"
```

### Running the demo

The demo script auto-detects whether the stack is running:

```bash
./demo/run.sh                          # uses default endpoints
./demo/run.sh http://localhost:50051   # explicit
```

### Stopping

```bash
docker compose down
# or:
just down
```

`down` stops and removes containers; volumes (and their data) persist unless you also pass `--volumes`.

## 5.3. What state survives across restarts?

Depends on how you started the kernel.

**In-memory mode** (default) — *nothing*. The kernel boots from the embedded ontologies on each start. Loaded files, registered capabilities, recorded traces, completed tasks: all gone.

**Persistent mode** (`--db <path>`) — layers, traces, registered WASM capabilities, and the embedded-ontology manifest all survive. The kernel rehydrates everything on restart. Details in [chapter 6](06-database-management.md).

**Docker Compose** — by default, no volume is mounted, so the container's RocksDB lives in the container's filesystem and is lost on `down`. To persist across container restarts, add a volume to the kernel service:

```yaml
services:
  kernel:
    # ...
    volumes:
      - ./data:/var/lib/eigenius
    command: ["serve", "--port", "50051", "--orchestrator", "http://orchestrator:8080", "--db", "/var/lib/eigenius"]
```

## 5.4. The four bootstrap layers, on every start

Whether in-memory or persistent, the kernel always boots with four embedded immutable layers in a parent-pointer chain:

```
core (root, no parent)
  └─ program
      └─ reflection
          └─ institution
```

These define `Class`, `Property`, `Program`, the `Let`/`Apply`/`Lambda` expression classes, the four epistemic categories (`DeclaredResource`, `ObservedResource`, `DerivedResource`, `VerifiedResource`), the `Institution` and `Comorphism` classes, and so on. Every loaded file or program references these via IRI.

The source-of-truth JSON files are under [`ontologies/`](../../../ontologies/):

- [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json) — the core
- [`ontologies/program/program-ontology.json`](../../../ontologies/program/program-ontology.json) — program model
- And similarly for reflection and institution

The bootstrap loader is [`kernel/src/bootstrap/`](../../../kernel/src/bootstrap/).

## 5.5. Connecting non-CLI clients

The kernel's gRPC service is defined in [`proto/`](../../../proto/). Any tonic-compatible client (or `grpcurl` for ad-hoc exploration) can talk to it:

```bash
# List services
grpcurl -plaintext localhost:50051 list

# Call an RPC
grpcurl -plaintext -d '{"iri":"urn:eigenius:core:Class"}' \
    localhost:50051 eigenius.kernel.EigeniusKernel/Inspect
```

The CLI's `--endpoint` mode is itself a thin wrapper over these RPCs ([`kernel/src/server/`](../../../kernel/src/server/)).

## 5.6. Network layout summary

| Process | Port | Protocol | Configurable |
|---|---|---|---|
| Kernel gRPC | 50051 | gRPC (HTTP/2) | `serve --port` |
| Orchestrator HTTP | 8080 | Connect RPC + REST `/health` + MCP | `EIGENIUS_ORCHESTRATOR_PORT` env in container |

Both services bind to all interfaces by default. For production, put a reverse proxy in front with TLS termination.

## 5.7. WSL 2 networking

WSL 2 forwards `localhost` to Windows automatically. After starting the stack inside WSL:

- From Windows browser: `http://localhost:8080/health` works
- From Windows applications: `localhost:50051` works for gRPC

If forwarding is intermittent (rare but possible after Windows updates), restart WSL:

```powershell
# Windows PowerShell, as admin
wsl --shutdown
```

Then restart your WSL session and the stack.

---

Next: **[6. Database management →](06-database-management.md)**
