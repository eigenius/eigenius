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

### Adding persistence

By default, the compose file runs the kernel in-memory. To persist across container restarts, mount a volume and add `--db` to the kernel's command:

```yaml
services:
  kernel:
    # ...
    volumes:
      - ./data:/var/lib/eigenius
    command:
      - "serve"
      - "--port"
      - "50051"
      - "--orchestrator"
      - "http://orchestrator:8080"
      - "--db"
      - "/var/lib/eigenius"
```

The `./data` host directory is created on first start. On subsequent `docker compose up`, the kernel rehydrates layers, traces, and capabilities from the persisted RocksDB.

For backups and restoration, use `docker compose exec kernel eigenius db export /var/lib/eigenius /tmp/export` followed by a `docker cp`. See [chapter 6](06-database-management.md).

### Rebuilding after code changes

Compose caches build layers aggressively. After changes:

```bash
# Rebuild the changed service
docker compose up --build kernel

# Or force a clean build of everything
docker compose build --no-cache
docker compose up -d
```

Build time is significant (full Rust workspace + WASM examples). For iterative development, prefer the three-terminal model from [chapter 5](05-running-locally.md).

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
| Key Vault | Stores `ANTHROPIC_API_KEY` and other secrets |
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

The `staging.bicepparam` and `production.bicepparam` files in `parameters/` carry environment-specific defaults (region, tier sizing, etc.). Customise these for your subscription before the first deploy.

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

1. **Azure Files volume mount** — declare a `Microsoft.App/managedEnvironments/storages` resource backed by an Azure Files share, then mount it into the kernel container at `/var/lib/eigenius`. The `kernel.bicep` module includes a hook for this; configure the storage account in your bicepparam.
2. **External managed RocksDB** — out of scope for the shipped templates. Run the kernel in stateless mode and persist to a side-car service.

For staging/dev environments, option (1) is the simplest path.

### Secret handling

`ANTHROPIC_API_KEY` (and any other secrets) is stored in Key Vault and exposed to the orchestrator container via a `secret` reference:

```bicep
// orchestration.bicep
secrets: [
  {
    name: 'anthropic-api-key'
    keyVaultUrl: '${keyvault.outputs.vaultUri}secrets/anthropic-api-key'
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

The container app's managed identity must be granted `Key Vault Secrets User` on the vault — wired up in `keyvault.bicep`.

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

// Bootstrap the four embedded ontology layers
let bootstrap_chain = bootstrap::bootstrap_layers()?;

// Add a custom layer
let mut builder = LayerBuilder::new("my-layer", Some(bootstrap_chain.clone()));
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

For ad-hoc exploration:

```bash
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext -d '{"iri":"urn:eigenius:core:Class"}' \
    localhost:50051 eigenius.kernel.EigeniusKernel/Inspect
```

For production clients, generate stubs from the `.proto` files and call them via your language's standard gRPC client library.

## 12.6. Deployment checklist

If you're deploying to Azure ContainerApps, *also* see the §12.2 caveat — the templates are a starting point that hasn't been validated end-to-end. Plan for iteration on the first deploy.

Before going live with a deployment:

- [ ] Set `--db <path>` and verify backup/restore works (export, restore, query)
- [ ] Configure the orchestrator with a real `ANTHROPIC_API_KEY` (or alternative LLM provider)
- [ ] Set CPU/memory limits sized for your workload (defaults are dev-sized)
- [ ] Set `minReplicas` if you don't want cold starts (Azure)
- [ ] Configure logging — both kernel and orchestrator log to stdout
- [ ] Verify the demo scripts run successfully against the deployed endpoints
- [ ] Set up monitoring on `http://<orchestrator>/health`
- [ ] Document the `ANTHROPIC_API_KEY` rotation procedure for your environment

---

Next: **[13. Troubleshooting and FAQ →](13-troubleshooting.md)**
