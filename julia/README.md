# Julia ecosystem code

Top-level home for Julia source that's part of the Eigenius project.
Mirrors `crates/` (Rust workspace) — different ecosystem, parallel
tree.

## Subdirectories

| Directory | Phase | What it is |
|-----------|-------|------------|
| [`runtime-worker/`](runtime-worker/) | 18d / 19a | Minimal Julia worker speaking the substrate's CBOR RPC. 18d uses it as an end-to-end capstone; 19a inherits it as the seed of `eigenius-julia`'s production worker. |

Future Phase 19+ subdirectories follow the same convention:

- `eigon-julia-gen/` — Julia mirror generator (D29 / 19b)
- `eigenius-julia-symbolics/` — Symbolics / ModelingToolkit institution (19d)
- `eigenius-julia-jump/` — JuMP institution (19e)

## Why a top-level directory rather than burying Julia under
`crates/runtime-substrate/assets/`

The Julia code is *real* Julia, not a bundled blob inside a Rust crate.
Each subdirectory is a proper Julia project (`Project.toml` +
`Manifest.toml` + `src/`) so it can be developed, tested, and pinned
the way the Julia ecosystem expects. Substrate-managed builds (Phase
18d / 19c) materialise these files into the OCI build context; Julia's
own tooling (`Pkg.instantiate`, `Test.runtests`) operates on the same
files in-place during dev.

## Manifest pinning

Each subdirectory's `Manifest.toml` is committed. Production deploys
build images via `Pkg.instantiate` (reads the pinned manifest, no
resolution, no network for resolution itself — only blob downloads).
Regenerate with:

```bash
julia --project=julia/<subdir> -e 'using Pkg; Pkg.update(); Pkg.precompile()'
```

…then commit the updated `Manifest.toml`. The substrate's image-build
pipeline picks up whatever's committed.
