# D57 Level-2 generator run (D60)

Lifts `concl_generator` from a Declared/mechanically-backed conformance leg to a
genuine **Derived** witness: the generator runs *through the kernel* as a program,
and the committed `ProgramTrace → IsDerivedAs(generate_result, GeneratorConforms)`
discharges the chain's `derived(...)` certificate.

## Mechanism (D60 — generic `oci` tool runtime)

`eigenius run` → kernel `RunProgram` → `RunRuntimeScript` component → orchestrator
→ substrate `OciToolRuntime`: spawns `eigenius-schemaorg-worker` in a pinned OCI
image (`lifecycle:Job`), provisions `gen_input` by `content_hash`, dispatches
`DispatchMethod`; the worker runs `eigenius_schemaorg::convert` and returns the
conversion-report `DerivedResource` (Eigon-CBOR) carrying
`canonical_proposition = GeneratorConforms("schema_org")`. The kernel commits it as
`obj:generate_result` under a `ProgramTrace`; the witness index mints `IsDerivedAs`.

Identical to the WRN `concl_vivo` lift (D56 §9), minus R's FFI mirror — the worker
is plain Rust linking the kernel.

## Verified without the full stack

- **`cargo test -p eigenius-oci --test oci_e2e -- --ignored`** — bakes the worker
  into a real image, spawns a sibling container, dispatches a real conversion, and
  asserts the report carries `GeneratorConforms("schema_org")`.
- **`cargo test -p eigenius-schemaorg --test d57_chain_validates`** — models the
  committed `generate_result` + `ProgramTrace` and asserts `concl_generator` Holds
  via `derived(...)`.

## Run it on the live compose stack — verified

`demo/d57-schema-org/run.sh` runs the whole thing end-to-end on a clean DB and shows all
five conclusions land `Holds` (incl. `concl_generator` via `derived(...)` and the thesis
`concl_main`). The steps it automates:

```bash
EIGENIUS_MOCK_LLM=true docker compose up -d        # orchestrator registers the `oci` runtime
                                                   # (EIGENIUS_OCI_* set in Dockerfile.orchestration)
./demo/d57-schema-org/run.sh
```

Two details the live run requires, both handled by the demo:

1. **Boot cross-check (D26 §9.3).** The `oci` image must be baked from the *same*
   `eigenius-schemaorg-worker` binary the orchestrator stages — else the worker's
   in-image manifest-hash won't match the one `OciToolRuntime` computes at dispatch and
   it exits 78. The demo `docker cp`s the staged binary, then `eigenius env build
   --language oci --worker-source-dir <that>` (which also emits the kernel-tracked
   `BuildRecipe`), and patches the resulting digest into `generate-program.json`'s
   `runtime:image_digest` (forwarded into the run's env by `synthesize_env`, so
   `OciToolRuntime` spawns the pre-built image — it does **not** build at dispatch; the
   orchestrator has no buildah).
2. **Input staging.** The pinned V30.0 input is staged into the depot's
   `extfile-cache/<sha256-hex>/<basename>` (the DooD-shared mount), so the substrate
   materializes it by content hash without a fetch.

Chain load order: `00–03`, `04a-evidence` (commits `gen_input`), **the run**,
`04b-conclusions`, `05-synthesis`.
