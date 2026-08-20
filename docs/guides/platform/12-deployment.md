# 12. Deployment

Deployment shapes covered in this chapter:

- **Docker Compose** — local or single-host. The fastest way to get the full stack running outside a developer environment. Used regularly; treated as the production-quality shape today.
- **Azure ContainerApps via Bicep** — *preliminary*. Templates exist in the repository as a starting point but have not been deployed end-to-end yet. See §12.2 for the caveat in detail.
- **Embedding the kernel as a library** — for advanced cases where the kernel runs inside another Rust process rather than as a separate gRPC service.

## 12.1. Docker Compose

The provided [`docker-compose.yml`](../../../docker-compose.yml) brings up both services with one command.

### Quick start

```bash
# Mock LLM (no API key needed)
EIGENIUS_MOCK_LLM=true docker compose up --build -d

# Real LLM
ANTHROPIC_API_KEY=sk-ant-... docker compose up --build -d

# Stop
docker compose down
```

The `just` recipes wrap the same:

```bash
just up-mock    # mock
just up         # real
just down       # stop
```

### Service definitions

The compose file declares two services:

- **kernel** — built from [`deploy/Dockerfile.kernel`](../../../deploy/Dockerfile.kernel). Exposes port 50051. Depends on `orchestrator` being healthy first.
- **orchestrator** — built from [`deploy/Dockerfile.orchestration`](../../../deploy/Dockerfile.orchestration). Exposes port 8080. Reads `EIGENIUS_MOCK_LLM` and `ANTHROPIC_API_KEY` from the host environment.

Both have health checks:

| Service | Health check |
|---|---|
| kernel | `eigenius --endpoint http://localhost:50051 inspect "urn:eigenius:core:Class"` |
| orchestrator | HTTP GET on `http://localhost:8080/health` |

The kernel waits for the orchestrator's health check to pass before its own command runs (`depends_on.condition: service_healthy`).

### Persistence

**The committed compose file already persists.** No edit is needed. The kernel service's command is:

```yaml
command: ["serve", "--port", "50051", "--orchestrator", "http://orchestrator:8080", "--db", "/var/lib/eigenius/db"]
volumes:
  - eigenius_db:/var/lib/eigenius/db
```

`eigenius_db` is a named Docker volume, not a bind mount, so the data lives under Docker's volume root rather than in the working tree. On each `docker compose up` the kernel rehydrates layers, traces and institution registrations from it. `docker volume rm eigenius_db` (with the stack down) is how you start clean.

To keep the database in a host directory instead, replace the named volume with a bind mount and keep the container-side path `/var/lib/eigenius/db` — the `--db` argument must match the mount point.

**Exporting cannot be done against the running stack.** `eigenius db export` opens RocksDB directly and the running kernel holds the exclusive directory lock, so `docker compose exec kernel eigenius db export ...` fails while the kernel is up. Stop the kernel first, then run the export in a throwaway container over the same volume:

```bash
docker compose stop kernel
docker run --rm -v eigenius_db:/var/lib/eigenius/db -v "$PWD/export:/export" \
    eigenius-kernel:local db export /var/lib/eigenius/db /export
docker compose start kernel
```

There is no `db import`; restoring an export is `eigenius load` over the exported files. See [chapter 6](06-database-management.md).

### Rebuilding after code changes

Compose caches build layers aggressively. After changes:

```bash
# Rebuild the changed service
docker compose up --build kernel

# Or force a clean build of everything
docker compose build --no-cache
docker compose up -d
```

Build time is significant (the full Rust workspace). For iterative development, prefer the three-terminal model from [chapter 5](05-running-locally.md).

## 12.2. Azure ContainerApps via Bicep

> **Status: preliminary, not yet exercised end-to-end.** The Bicep
> templates are committed to the repository as a starting point and are
> believed to be syntactically correct, but no Eigenius deployment has
> been validated against a live Azure subscription. Treat the section
> below as a structural reference, not a runbook. Expect to iterate on
> sizing, identity, and storage configuration when you stand it up the
> first time.

The repository ships with Azure Bicep templates for a managed cloud deployment. Files under [`deploy/bicep/`](../../../deploy/bicep/):

```
deploy/bicep/
├── main.bicep                          orchestrating template
├── modules/
│   ├── acr.bicep                       Azure Container Registry
│   ├── environment.bicep               ContainerApps managed environment
│   ├── kernel.bicep                    kernel ContainerApp
│   ├── orchestration.bicep             orchestrator ContainerApp
│   └── keyvault.bicep                  Key Vault for secrets
└── parameters/
    ├── staging.bicepparam              staging environment overrides
    └── production.bicepparam           production environment overrides
```

What gets provisioned by `main.bicep`:

| Resource | Purpose |
|---|---|
| Container Registry | Holds the kernel and orchestrator images |
| Key Vault | Created, but orphaned — nothing writes secrets to it and nothing reads them. See "Secret handling" below. |
| ContainerApps managed environment | The host environment for both services |
| Kernel ContainerApp | Runs `eigenius serve` |
| Orchestration ContainerApp | Runs the Deno orchestrator |
| Managed identities | For ACR pull and Key Vault read access |

### Deploying

Prerequisites: an Azure subscription, the `az` CLI logged in, a target resource group.

```bash
# Build and push images to ACR
docker build -t <acr>.azurecr.io/eigenius-kernel:<tag> -f deploy/Dockerfile.kernel .
docker build -t <acr>.azurecr.io/eigenius-orchestration:<tag> -f deploy/Dockerfile.orchestration .
az acr login --name <acr>
docker push <acr>.azurecr.io/eigenius-kernel:<tag>
docker push <acr>.azurecr.io/eigenius-orchestration:<tag>

# Deploy
az deployment group create \
    --resource-group <rg> \
    --template-file deploy/bicep/main.bicep \
    --parameters @deploy/bicep/parameters/staging.bicepparam \
    --parameters imageTag=<tag>
```

The `staging.bicepparam` and `production.bicepparam` files in `parameters/` carry three parameters each — `environment`, `imageTag` and `acrLoginServer`. No region, no tier sizing: location comes from the resource group and CPU/memory are hardcoded in the modules. Set `acrLoginServer` to your own registry before the first deploy.

### Updating

For container image updates:

```bash
# Build + push new image
docker build -t <acr>.azurecr.io/eigenius-kernel:<new-tag> -f deploy/Dockerfile.kernel .
docker push <acr>.azurecr.io/eigenius-kernel:<new-tag>

# Re-run deployment with new tag
az deployment group create \
    --resource-group <rg> \
    --template-file deploy/bicep/main.bicep \
    --parameters @deploy/bicep/parameters/staging.bicepparam \
    --parameters imageTag=<new-tag>
```

ContainerApps performs a rolling update — old replicas drain while new ones come up.

### Persistent storage in Azure

ContainerApps doesn't ship native persistent volumes for the `ContainerApp` workload type. Two options:

1. **Azure Files volume mount** — declare a `Microsoft.App/managedEnvironments/storages` resource backed by an Azure Files share, then mount it into the kernel container at `/var/lib/eigenius`. You will have to write this yourself: `kernel.bicep` declares no volume, no volume mount and no storage resource, and its container has no `command`, so the kernel runs the image default — no `--db`, and therefore in-memory. Adding persistence means adding the storage resource, the mount, *and* a `command` that passes `--db`.
2. **External managed RocksDB** — out of scope for the shipped templates. Run the kernel in stateless mode and persist to a side-car service.

For staging/dev environments, option (1) is the simplest path.

### Secret handling

**Not wired up.** `keyvault.bicep` creates an RBAC-authorized vault and outputs `vaultUri`; `main.bicep` passes that output to nothing. `orchestration.bicep` declares no `secrets:` block and no `secretRef`, and no role-assignment resource exists anywhere under `deploy/bicep/`. Nothing puts `ANTHROPIC_API_KEY` into the vault and nothing reads it out — the orchestrator app gets the key only from whatever plain `env` value you set.

Wiring it up means adding all three pieces yourself. The shape:

```bicep
// orchestration.bicep — none of this is in the committed template
secrets: [
  {
    name: 'anthropic-api-key'
    keyVaultUrl: '${vaultUri}secrets/anthropic-api-key'
    identity: 'system'
  }
]
env: [
  {
    name: 'ANTHROPIC_API_KEY'
    secretRef: 'anthropic-api-key'
  }
]
```

plus a `Microsoft.Authorization/roleAssignments` granting the app's system-assigned identity `Key Vault Secrets User` on the vault, plus a `vaultUri` parameter threaded from `main.bicep`.

### Cost considerations

- ContainerApps default-scales to zero when idle if you set `minReplicas: 0`. The first request after a quiet period pays a cold-start penalty (10–30 seconds). For most usage, set `minReplicas: 1` to keep one warm replica.
- Persistent storage (Azure Files) is billed separately.
- ACR has tier-based pricing; Standard is sufficient for typical image sizes.

## 12.3. Embedding the kernel as a library

For consumers that want to embed the kernel directly in another Rust application — without running it as a separate gRPC service — the kernel is published as a Cargo crate.

Add to your `Cargo.toml`:

```toml
[dependencies]
eigenius-kernel = { path = "<repo>/kernel" }   # or git/version dep
```

Use the API directly:

```rust
use eigenius_kernel::bootstrap;
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::ontology::{eigon_json, Iri};
use eigenius_kernel::query;
use std::sync::Arc;

// Bootstrap the twenty embedded ontology layers; the returned
// ExecutionContext is headed at the tip of that chain.
let ctx = bootstrap::bootstrap()?;
let head = ctx.head().clone();

// Add a custom layer
let mut builder = LayerBuilder::new("my-layer", Some(head));
let resources = eigon_json::parse_document(my_json_str)?;
for r in resources {
    builder.add_resource(r)?;
}
let layer = Arc::new(builder.build()?);

// Run a query
let result = query::execute(
    "USING \"urn:eigenius:core:Class\" MATCH Class(?c) { short_name: ?n } RETURN [] { name: ?n }",
    &layer
)?;
```

This sketch is illustrative, not compiled as a doctest — check the current signatures in `kernel/src/bootstrap/mod.rs` and `kernel/src/query/` before copying it.

The CLI binary itself is a thin user of this API ([`cli/src/main.rs`](../../../cli/src/main.rs)). Embedding gives you direct in-process access at the cost of accepting Rust as your application language.

## 12.4. Running without an orchestrator

For deployments where you only need read-only operations (queries, file inspection, type-check) and no IO components, the orchestrator is unnecessary. Run the kernel without `--orchestrator`:

```bash
eigenius serve --port 50051 --db /var/lib/eigenius
```

CLI commands `load`, `query`, `inspect`, `program-validate` continue to work. `run` works for programs whose bodies don't dispatch IO components — for example, programs that only manipulate resources structurally without calling `CompleteText` or `CompleteJson`.

This deployment shape is suitable for read-heavy workloads (pure-data services), embedded analytics, or institutions that don't depend on LLM dispatch.

## 12.5. gRPC clients beyond the CLI

The kernel's gRPC service (defined in [`proto/`](../../../proto/)) is consumable by any tonic-compatible Rust client or any standard gRPC client (Python, Go, TypeScript, etc.) generated from the protobuf definitions.

**`grpcurl` needs the `.proto` files.** The kernel registers no server-reflection service — `tonic-reflection` is not a workspace dependency — so `grpcurl -plaintext localhost:50051 list` fails with an "server does not support the reflection API" error. Pass the protos explicitly instead. The package is `eigenius.v1` and the service is `EigeniusKernel`, so the fully-qualified method is `eigenius.v1.EigeniusKernel/Inspect`:

```bash
grpcurl -plaintext -import-path proto -proto eigenius.proto \
    localhost:50051 list

grpcurl -plaintext -import-path proto -proto eigenius.proto \
    -d '{"iri":"urn:eigenius:core:Class"}' \
    localhost:50051 eigenius.v1.EigeniusKernel/Inspect
```

For production clients, generate stubs from the `.proto` files and call them via your language's standard gRPC client library.

## 12.6. Security: nothing in this stack is secured

Read this before exposing any of it.

- **No TLS.** The kernel's tonic server is built with `accept_http1(true)`, a gRPC-Web layer and raised message-size limits, and nothing else — no `ServerTlsConfig`, no identity. The orchestrator's `Deno.serve` call takes a port and a handler.
- **No authentication.** There is no interceptor, bearer-token check or API-key check on either listener. The TypeScript SDK accepts a `bearerToken` option whose own doc comment says it is unused, and the constructor never reads it.
- **No authorization.** `kernel/src/capability/` validates and registers chain-declared institutions. It grants nothing to nobody. Every RPC is available to every caller.
- **Both ports reach every interface.** Kernel and orchestrator both bind `0.0.0.0`, and the compose file publishes both with the short `"PORT:PORT"` form. No `127.0.0.1` binding appears anywhere in the tree.
- **The orchestrator container mounts the host Docker socket**, so the runtime substrate can spawn sibling worker containers. Combined with the four points above, an unauthenticated request to port 8080 is a path to root-equivalent control of the host Docker daemon.

The Azure templates change none of this: the orchestration app declares external ingress on 8080 over plain HTTP with no IP restrictions, no client-certificate mode and no auth configuration, and the kernel app declares internal ingress on 50051, unauthenticated to anything else inside the environment. Container Apps terminates TLS at its own edge FQDN as a platform default — that is the only TLS in the story, no committed file configures it, and it authenticates nobody.

**The deployment assumption is a trusted network.** Run the stack where the ports are reachable only by trusted callers: loopback, a private segment, or behind a reverse proxy that terminates TLS and authenticates before forwarding. Do not publish either port to the internet.

## 12.7. Deployment checklist

If you're deploying to Azure ContainerApps, *also* see the §12.2 caveat — the templates are a starting point that hasn't been validated end-to-end. Plan for iteration on the first deploy.

Before going live with a deployment:

- [ ] Set `--db <path>` and verify backup/restore works (export, restore, query)
- [ ] Configure the orchestrator with a real `ANTHROPIC_API_KEY` (or alternative LLM provider)
- [ ] Set CPU/memory limits sized for your workload (defaults are dev-sized)
- [ ] Check `minReplicas` for your workload. The templates set 1 for the orchestrator and 1 (staging) or 2 (production) for the kernel — there is no scale-to-zero configuration to undo, and no cold-start problem to fix
- [ ] **Put the deployment behind something that authenticates.** See §12.6
- [ ] Configure logging — both kernel and orchestrator log to stdout
- [ ] Verify the demo scripts run successfully against the deployed endpoints
- [ ] Set up monitoring on `http://<orchestrator>/health`
- [ ] Document the `ANTHROPIC_API_KEY` rotation procedure for your environment

---

Next: **[13. Troubleshooting and FAQ →](13-troubleshooting.md)**
