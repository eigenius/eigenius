# D53 — implementation plan

> Detailed, codebase-mapped plan to implement [D53 large-data tracking](../design/d53-large-data-tracking.md).
> Components mapped to concrete changes per crate / module / service, with file
> anchors and a phased order. Grounded in a codebase survey (June 2026); line
> numbers are approximate and drift with edits.

## 0. Shape recap (what we're building)

D53 is an **input layer**, not an institution (D53 §7): an **ontology extension**
(`PinnedExternalFile` + `DatasetSchema` nodes) plus a **substrate capability**
(fetch external file by `reference` → verify `content_hash` → materialize into the
worker). The compute layer above it (native D52 or wrapped D56) is chosen by
*method*, not by where the bytes live (D53 §6). So the build is mostly
**substrate-side**, with a small ontology + CLI surface and *no kernel
correctness change* for the read path.

**Design lever that keeps v1 small:** the kernel already sends a component's
`input` as a (small) CBOR resource. A `PinnedExternalFile` node *is* small
(reference + hash + schema). So the kernel sends it unchanged; the **substrate
facade detects an `is_a PinnedExternalFile` input, resolves + materializes it, and
hands the worker a path** — no kernel/proto change for v1.

## 1. Component → location map

| D53 component | Crate / service | Module (file:line anchor) | Change |
|---|---|---|---|
| `PinnedExternalFile` + `DatasetSchema` classes | ontologies | **new** `ontologies/ingest/ingest-ontology.json` | author classes/properties (JSON shape mirrors `ontologies/runtime/runtime-substrate-ontology.json`) |
| Bootstrap registration | kernel | `kernel/src/bootstrap/mod.rs` — `bootstrap_with_storage()` (~290) + `embedded_ontologies()` array (~595-660) | add a `load_layer("ingest", …)` after `runtime`; add the tuple to the embedded array (drives the manifest hash) |
| Content-addressed IRI | `eigenius-runtime-substrate` | `crates/runtime-substrate/src/content_address.rs:29-95` | add `PinnedExternalFileIdentity { reference, content_hash, media_type }` → `content_addressed_iri()` (clone the `RuntimeScriptIdentity` length-prefixed-hash pattern) |
| External-file **resolver** (file:// ✅, oxen:// ✅) | `eigenius-runtime-substrate` | `crates/runtime-substrate/src/external_file.rs` (+ `oxen.rs`) | scheme dispatch → fetch → **stream-verify** `content_hash` (fail closed) → materialize into the depot cache. Both backends done; large files never buffered in memory |
| Content-hash verify | `eigenius-runtime-substrate` | reuse `content_address.rs` (`sha2`) + `types.rs:37-56` (`ImageDigest::parse` sha256-hex shape) | `verify_file_hash(path, expected) -> Result<(), _>` |
| Oxen fetch (subprocess) ✅ | `eigenius-runtime-substrate` | `oxen.rs` | shell out to the `oxen` CLI: `oxen download <ns/repo> <path> --output <dir> --revision <rev> --host <host> --scheme <s>`; auth via inherited `$OXEN_CONFIG_DIR`/`auth_config.toml` (D53 §10). `file://` = `std::fs::copy` into the cache |
| Depot cache (no new mount) ✅ | `eigenius-runtime-substrate` | `external_file.rs` cache root, set in `orchestration/runtime-substrate-native/src/lib.rs` register_* fns | **Refinement:** materialize under `<depot>/extfile-cache/<sha256-hex>/<name>`. The whole depot is *already* RO-bind-mounted into the worker at the same path (`container.rs:145-151`), so the worker sees the cache with **no per-file `Mount`**. Cache root threaded via `SubstrateDispatcher::set_extfile_cache_root`; `None` ⇒ same-host passthrough. Content-addressed ⇒ idempotent (cache hit skips re-fetch+re-verify) |
| Provision: detect file input → materialize → pass path ✅ | `eigenius-runtime-substrate` | `crates/runtime-substrate/src/facade.rs` — `prepare_input` applied at all three input parse sites (`dispatch_run_runtime_script`, `dispatch_call_runtime_method`, `dispatch_external_institution`) | if `input.is_a` contains `PinnedExternalFile`: resolve+materialize, synthesize a `{ ingest:materialized_path }` input resource for the worker. Lean shape — no RPC/proto change |
| Boundary gate (eager verify) | `eigenius-runtime-substrate` | `crates/runtime-substrate/src/boundary.rs:114-170` (`run_check`) | optional pre-dispatch fetch+verify gate |
| Worker reads the path (R ✅, Julia deferred) | `eigenius-r-worker` + R driver | `crates/eigenius-r-worker/src/ffi.rs` `r_eigon_materialized_path` + `r/EigeniusRWorker.R` doc | R done. **Julia deferred** — Julia's input model is typed-mirror-dispatch (`discover_decoders`), not raw-property read; the natural equivalent is a `PinnedExternalFile` mirror struct in `EigeniusJuliaCommon`, to add when a Julia external-file consumer exists. The substrate provision is already language-agnostic (Julia dispatches get `ingest:materialized_path` for free) |
| Error taxonomy ✅ | `eigenius-runtime-substrate` (+ kernel mirror) | `crates/runtime-substrate/src/error.rs` | `ExternalFetchFailed { reference, reason }`, `ContentHashMismatch { reference, expected, got }` |
| `eigenius data` CLI | `eigenius-cli` | **new** `cli/src/data.rs` + `cli/src/main.rs` (Commands enum ~330, dispatch ~948, `remote_data` ~3124) | `attach`/`list`/`inspect`/`verify`, mirroring `scripts.rs`; reuse `common.rs::{fetch_resource, submit_resource_for_load}` + `run_query`/`connect_client` |
| Oxen CLI in the image ✅ | deploy | `deploy/Dockerfile.orchestration` | installs the `oxen` CLI from the per-arch release `.deb` (`oxen-linux-{x86_64,arm64}.deb`, pinned `OXEN_VERSION` arg + `dpkg --print-architecture` detection) + sets `OXEN_CONFIG_DIR` |
| napi registration (no-op if substrate-internal) | orchestration | `orchestration/runtime-substrate-native/src/lib.rs:254-277` | only if a new dispatch param is needed (avoided by the lean design) |

## 2. Phased plan

### Phase 0 — Ontology + identity + attach (pure graph, no execution)

*Goal: track files as typed nodes; nothing runs yet.*

1. **`ontologies/ingest/ingest-ontology.json`** — `ingest:PinnedExternalFile` (`is_a reflection:ObservedResource`; requires `reference`, `content_hash`, `media_type`; recommends `schema`, `content_encoding`, descriptive metadata → defer names to D57), `ingest:DatasetSchema`, `ingest:Dimension`, `ingest:Measure`, `ingest:Attribute`, `ingest:Layout`, and their properties. Start minimal: the §4 *cube* fields; the §10 refinements (plural `schemas`, `Collection` layout) can be additive later.
2. **`kernel/src/bootstrap/mod.rs`** — register the layer (after `runtime`) + add to `embedded_ontologies()`.
3. **`content_address.rs`** — `PinnedExternalFileIdentity::content_addressed_iri()`.
4. **`cli/src/data.rs`** — `data attach <file://path>` (read local bytes, compute sha256, mint IRI, build the node, `submit_resource_for_load`); `data list`/`data inspect` (query/fetch). Defer `oxen://` + `verify` to Phase 2.
5. **Tests:** `cargo test` for the identity hash; an attach→inspect round trip against an in-memory kernel (mirror `scripts.rs` test patterns). `cargo build` proves the ontology loads (the bootstrap manifest must stay green).

*Unblocks:* files are first-class typed graph nodes. No substrate work yet.

### Phase 1 — Resolver + provision (the read path, `file://`)

*Goal: a `RunRuntimeScript` reads a `file://` `PinnedExternalFile`.*

1. **`crates/runtime-substrate/src/external_file.rs`** (new) — `resolve_and_materialize(file: &Resource, depot: &Path) -> Result<PathBuf, RunError>`: parse `reference` scheme; `file://` → resolve against the configured volume; compute sha256; compare to `content_hash` (fail closed); place/symlink into `<depot>/cache/<sha256>/<name>`.
2. **`spawner/docker/container.rs:134-177`** — extend the `mounts` vec with a read-only bind of the cache dir (or rely on it already being under the depot mount — confirm path is depot-relative, then no new mount needed).
3. **`facade.rs:139-159`** — in `dispatch_run_runtime_script`, after `parse_resource(input_cbor)`: if the input `is_a` `ingest:PinnedExternalFile`, call `resolve_and_materialize`, then synthesize the worker input resource `{ is_a: …, ingest:materialized_path: "<path>" }` (lean design — reuses the existing `inputs: Vec<ByteBuf>` channel; **no RPC/proto change**).
4. **`eigenius-r-worker/src/ffi.rs`** — `r_eigon_materialized_path(input_cbor) -> SEXP` (string); the R driver/script opens it.
5. **Tests:** an integration test (under `LocalSpawner`) running a trivial R script that reads a `file://`-attached CSV and emits a `DerivedResource`. Reuse `test_runtime_docker.rs` patterns.

*Unblocks:* the whole read path end-to-end for local-volume files — and this is exactly how a DepMap matrix on a mounted volume would feed limma.

### Phase 2 — Oxen backend + auth + `verify` ✅

1. **`oxen.rs`** (new) ✅ — `oxen://[<host>/]<ns>/<repo>@<revision>/<path>` parse + `oxen download` command build (pure, unit-tested) + subprocess `download_into` + `render_auth_config_toml`/`write_auth_config`. Uses the prebuilt CLI (not `liboxen` — ~160 deps). Binary/scheme overridable via `EIGENIUS_OXEN_BIN`/`EIGENIUS_OXEN_SCHEME`. **`external_file.rs`** ✅ — `oxen://` arm stages the download into a per-hash cache subdir, **stream-hashes** (large files never buffered), verifies fail-closed, atomically renames; content-addressed cache hit skips re-fetch.
2. **`deploy/Dockerfile.orchestration`** ✅ — installs the `oxen` CLI (pinned `OXEN_DEB_URL` build arg) in the orchestrator image; sets `OXEN_CONFIG_DIR` for the auth secret.
3. **`cli/src/data.rs`** ✅ — `data attach` is scheme-aware (`oxen://` fetches once to hash; local/`file://` stream-hash) + `data verify <iri>` (re-fetch + recompute + check, fail closed). Live-verified: file:// verify passes intact, exits 1 on tamper.
4. **Distribution** (D53 §3.1) ✅ — `ResolveOptions::reject_node_local_files` (set via `SubstrateDispatcher::set_reject_node_local_files`) rejects node-local `file://` in a distributed deployment; `oxen://` is the per-host pull path.
5. **Tests:** ✅ pure parse/args/auth unit tests; cache idempotence + fail-closed (file:// **and** cache path) unit-tested with tampered files; an `#[ignore]`'d `oxen_live_download_verifies` e2e (env-driven, needs CLI + server).

*Unblocks:* large/versioned/distributed data — the production path.

### Phase 3 — `DatasetSchema`-driven typing

1. The worker (or a native consumer) reads **typed** columns/rows per the schema's dimensions/measures/layout, not just raw bytes — the mirror-generator analog (`crates/runtime-substrate/src/mirror_generator.rs`) extended to external-file schemas.
2. Implement the §10 refinements as needed: **plural `schemas`** (member/sheet selector — the `.rds`/multi-sheet case), the **`Collection`** layout (`.gmt`), code-list resolution, layout orientation.
3. Schema vocabulary IRIs route through **D57** (`urn:schema_org:` mapping) — settle that first if the descriptive metadata fields are wanted typed.

*Unblocks:* the full WRN corpus modeling ([worked example](d53-wrn-attachment-worked-example.md)).

### Phase 4 — Native recompute over a file (D52 + D53)

1. Extend `eigenius-statistics` so a `SampleSet` whose values reference a `PinnedExternalFile` is materialized (Phase 1 path) and the deterministic numerics read the materialized array — native-grade at scale (D53 §6). Decide execution placement: in-kernel read of a verified local array, or a substrate-dispatched native-Rust component (keeps kernel I/O-free).

*Unblocks:* the storage⊥grade promise — native warrants over genome-scale data.

### Phase 5 — limma D-DIFF (the payoff, P7)

1. A wrapped-R limma `RuntimeScript` reading the Achilles/DRIVE matrices as `PinnedExternalFile` inputs → commits the differential-dependency result → lifts `dd_achilles`/`dd_drive` from linked-external to reproduced-external. Pure composition of Phases 1–2 + D56.

## 3. Cross-cutting / decisions before coding

- **Multi-input.** The facade passes a single `input` (`&[input]`, `facade.rs:157`). The multi-file join (WRN RecQ) and limma's matrix+sample-info need *multiple* `PinnedExternalFile` inputs → a real extension (kernel sends N inputs; RPC `inputs: Vec<ByteBuf>` already plural, but the kernel/program model sends one). Scope: defer to when a multi-file analysis lands, or do it in Phase 1 if limma needs it (it joins matrix + sample-info).
- **Lean vs explicit worker channel.** Phase 1 uses the *lean* path (synthesize a `materialized_path` input resource — no proto/RPC change). The *explicit* channel (proto `ComponentRequest.file_inputs`, RPC `DispatchMethod.file_inputs`, worker `r_input_file_path`) is cleaner long-term but heavier; adopt it only if the lean path proves limiting.
- **`LibraryContent::External` is stubbed** (`mirror_generator.rs`, rejected in `eigenius-julia/src/runtime.rs:809`). D53's resolver should *generalize* that discipline (fetch+verify) rather than fork it; consider unifying the mirror-external path and the input-file path through one resolver.
- **Depot under DooD** (`spawner/docker/depot.rs`): the cache must live *under* the depot host path so the DooD bind-mount makes it visible at the same path in the worker. Verify `verify_tempdir_under_depot` semantics apply to the cache dir.
- **Schema vocabulary** depends on **D57**; Phases 0–2 only need `reference`/`content_hash`/`media_type` (no schema), so D57 isn't blocking until Phase 3.

## 4. Suggested first cut

**Phases 0 + 1** are the minimal vertical slice: attach a `file://` file as a typed node and have a `RunRuntimeScript` read it — no Oxen, no schema, no kernel change. That proves the seam end-to-end and is exactly the shape limma needs (Phase 5) once Oxen (Phase 2) lands. Everything else is additive.
