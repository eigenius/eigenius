# D55 — R / Bioconductor language runtime (with mirror)

*Status: design (2026-06-13). Specifies a production R language runtime for the Eigenius substrate, at feature parity with `eigenius-julia` / `eigenius-lean`: a worker, a deterministic pinned image, and a mirror generator. Motivated by the WRN-helicase Tier-2 recomputes (limma moderated-t, fgsea, lme4) but built once as a general R/Bioconductor capability — the long-term payoff for a life-sciences platform.*

## 1. Why R, why now

The WRN encoding has driven every institution-recomputable warrant to kernel-recomputed (C-WRN, D-REFINE, D-RECQ, D-BIOM, p53, C-VAL, C-MECH cell-cycle/apoptosis, C-MMR, the Fig 2c rescues). What remains linked-external is the **external-tool frontier** — methods the authors ran in R/Bioconductor that we do not reimplement:

- **D-DIFF** (`dd_achilles` / `dd_drive`): genome-wide differential dependency, **limma** `voom`+`lmFit`+`eBayes` moderated-*t* → WRN is the top hit.
- **GSEA** (Fig 3a, feeds `mech_rule`): **fgsea** gene-permutation enrichment over MSigDB Hallmark on the RNA-seq differential expression.
- **C-VIVO xenograft** (`vivo_xenograft`, `vivo_seed_control`): **lme4** random-slope mixed-model LRT on KM12 tumor-growth.

The decision (this thread): **wrap the real R tools** rather than reimplement them in Rust. Rationale — faithful by construction (it *is* limma/fgsea/lme4 at a pinned version, not a re-derivation whose fidelity must be argued), and a reusable R/Bioconductor capability compounds across future life-sciences encodings. The reimplementation path (native Rust numerics) was considered and rejected for these three: limma's empirical-Bayes (`squeezeVar`/`fitFDist`) and lme4's REML carry a real fidelity burden, and the bit-faithful target is the R package itself.

## 2. Warrant grade (vs D52, aligned with D53)

This is the same grade distinction D53 drew for ingestion:

> **Recompute the science natively where the claim is worth independent re-derivation; pin-and-reproduce the external tool where the trust root is "the authors' actual program, in a pinned environment, on pinned inputs, producing a pinned output that anyone can reproduce."**

A wrapped-R warrant is **"faithful, reproducible execution of a pinned external tool"** — weaker than D52's "proven by re-derivation," stronger than "agent-attested." It is the right grade for limma/fgsea/lme4: the community trusts these implementations; what was missing was a re-runnable, content-addressed execution record. That is exactly what a `RuntimeInvocation` provides.

## 3. The CBOR/worker problem is already solved — reuse the Lean pattern

R has no mature CBOR library and no base unix-domain-socket support. This is **not** a blocker, because `eigenius-lean-worker` already solved the identical problem for Lean:

- `eigenius-lean-worker` is a **Rust cdylib** that hosts the UDS transport + length-prefixed CBOR framing (`runtime_substrate::rpc::codec`, `MAX_FRAME_SIZE_DEFAULT`) and the workspace Eigon-CBOR codec (`eigenius_kernel::ontology::eigon_cbor::{serialize_resource, parse_resource_lenient}`), exposing a **C ABI** (`c/lean_bridge.{c,h}`, `lean_ffi.rs`).
- Lean drives the dispatch loop and does its computation via FFI into the Rust worker; it never sees a socket or a CBOR byte. "The cdylib hosts the workspace's Eigon-CBOR codec so the [foreign] worker [doesn't need] a parallel CBOR implementation."

**R inherits this verbatim.** R has first-class C FFI (`.Call`, `dyn.load`, Rcpp). `eigenius-r-worker` is a Rust cdylib — a near-clone of `eigenius-lean-worker` — and an R driver loads it and drives the loop, calling into R (limma/fgsea/lme4) for the computation. The protocol, framing, codec, cross-check, and witness assembly are **shared Rust**, identical across Julia/Lean/R. The only R-specific code is the FFI driver + the data marshalling (§5).

## 4. Crate layout (mirrors Julia + Lean)

| Crate | Analogue | Responsibility |
|---|---|---|
| `eigenius-r` | `eigenius-julia` | `RLanguageRuntime: LanguageRuntime` (dispatch lifecycle, spawn, `run_script`/`call_method`), `dockerfile.rs` (R image fragments), `conventions.rs` (shared IRIs/paths), re-export of the mirror generator. |
| `eigenius-r-worker` | `eigenius-lean-worker` | Rust cdylib: UDS+CBOR framing (reuse `rpc::codec`), Eigon-CBOR codec (reuse `eigon_cbor`), C ABI worker loop + decode/encode helpers, host↔container manifest cross-check. |
| `eigenius-r-runtime` | `eigenius-lean-runtime` | `mirror_gen.rs`: walk the ontology layer → emit R type representations → commit `RPackageMirror` → bake precompiled into the env image. |
| `r/` (workspace dir) | `julia/`, lean project | R-side driver (`EigeniusRWorker.R` loading the cdylib + dispatch loop), the worker `DESCRIPTION`/`renv.lock`, mirror packages. |

`eigenius-r` registers with the existing `SubstrateDispatcher.register_language_runtime`; `language = "r"` on `RuntimeScript`/`RuntimeEnvironment`/`RuntimeMethodSignature` routes to it. No orchestration changes — it slots into the registry exactly as Julia and Lean do.

## 5. R-side specifics (the only genuinely new work)

- **Driver.** `EigeniusRWorker.R` `dyn.load`s the `eigenius-r-worker` cdylib and runs the loop: `worker_next_request_kind()` → on `dispatch_method`, fetch the target (R source / method name) + inputs (CBOR bytes), decode via the Rust helpers, `eval` / call, encode the result, `worker_send_dispatch_ok()`. Pattern-identical to `JuliaWorker.jl`'s loop and the Lean FFI driver.
- **Marshalling.** Eigon `Resource` ↔ R value. The common cases for our targets: a `SampleSet`/matrix → R `matrix`/`data.frame`; an output `topTable`/GSEA table/LRT result → an Eigon `DerivedResource` with typed properties. The Rust worker exposes decode helpers (like Lean's `worker_decode_eigon_string_property`); R-specific helpers add `decode_eigon_matrix` / `encode_dataframe_as_resource`. Numeric columns are `f64`; row/col names are string arrays.
- **Determinism.** R/limma/lme4 are deterministic given inputs. **fgsea uses permutations** — pin the RNG seed (`set.seed`) and the `nperm`/`eps` so the enrichment p is reproducible; record the seed in the `RuntimeInvocation.numerical_metadata`. (Equivalently use fgsea's deterministic multilevel mode.) Without a pinned seed the warrant is not bit-reproducible — fail closed on its absence for GSEA.

## 6. The image and version pinning (the payoff)

- **Base + packages.** `RImagePlan { base_digest, bioc_release, packages: [limma@x, fgsea@y, lme4@z, ...] }` → `dockerfile_fragments` → deterministic `buildah` build (D26 §9.2) → captured `ImageDigest` stored on `RuntimeEnvironment.image_digest`. Base is a digest-pinned `bioconductor/bioconductor_docker@sha256:…` (or `rocker/r-ver`).
- **Lockfile.** `renv.lock` (R's lockfile) is the package-version manifest; its hash is the host↔container cross-check anchor (`cross_check.rs`), exactly as Julia uses `Manifest.toml`. The worker refuses to start if its in-image manifest hash disagrees with the chain's `RuntimeEnvironment`.
- **What the digest pins.** R version + Bioconductor release + every transitive dependency, cryptographically. A warrant cites "computed with limma `<v>` / fgsea `<v>` / lme4 `<v>` in image `sha256:…`" and is reproducible bit-for-bit by anyone with that image. This is the **faithful-by-construction** property and the long-term life-sciences payoff: every future R/Bioconductor analysis pins exactly the tool the authors used.
- **Graded deployment.** `LocalSpawner` (host `Rscript`, `image_digest: None`) is the dev/prototype path — get the wrapping working end-to-end before building the OCI image. `DockerSpawner` with the pinned image is the production path that yields the reproducibility guarantee. (D26 §10.1 deployment shape (c).)

## 7. The mirror (feature parity)

`eigenius-r-runtime::mirror_gen` is the R analogue of the Julia/Lean mirror generators: it walks an Eigenius ontology layer and emits **R representations of Eigenius types** so the worker can do typed dispatch over `RuntimeMethodSignature` inputs (the `CallRuntimeMethod` path, beyond bare `RunRuntimeScript`). Specifics for R:

- **Type representation.** R's type system is weaker than Julia's multiple dispatch or Lean's dependent types. The mirror emits **S4 classes** (formal slots + validity) for Eigenius resource types, with `setGeneric`/`setMethod` for the method registry — the closest R idiom to typed dispatch. (S4, not R6/S3: S4 has formal slot typing and a validity contract the mirror can populate from the ontology.)
- **Output.** An `RPackageMirror` resource (the generated R package source tree + `DESCRIPTION`), committed to the chain and baked precompiled into the env image, with the mirror modules loaded at worker boot — mirroring `JuliaPackageMirror` (19a.3) and the Lean `RegisterMirror` flow.
- **Boundary check.** The worker validates the in-image mirror version against the chain's mirror IRI at dispatch (the Lean `MirrorVersionMismatch` / Julia mirror-hash cross-check pattern).
- **Scope note.** For the WRN Tier-2 targets, the bare `RunRuntimeScript` path (a `RuntimeScript` of R source over chain-resident inputs) is sufficient — the mirror lights up the *typed `call_method`* path. It is built for parity and to make R a first-class typed runtime, not because the three recomputes require it.

## 8. Execution model for the WRN warrants (lightweight, not institutions)

Per the decision in this thread, the Tier-2 recomputes are **`RuntimeScript` dispatches, not full institutions**:

```
RuntimeScript (R source)  +  RuntimeEnvironment (ImageDigest)  +  inputs (chain-resident Eigon-CBOR)
        │  dispatch_run_runtime_script
        ▼
DerivedResource (the R output: topTable / GSEA table / LRT result)
        │  facade stamps reflection:DerivedResource + emits InstitutionEmittedDerivation → D49 admits IsDerivedAs
        ▼
RuntimeInvocation { script, environment, inputs, output, image_digest, timestamps, numerical_metadata }
        │  a thin Declared bridge reads the one load-bearing value
        ▼
domain conclusion (lemma-citable, D54)
```

No `AnalysisPlan` class / QueryClass gate / canonical-proposition apparatus per method. Three R scripts + three Declared bridges:

- **xenograft** → `lme4` LRT p < α (DOX vs no-DOX slope) → bridge → `InVivoDependence(WRN, MSI)` ⇒ lift `concl_vivo`; the C911 LRT → `SeedControlInert` (existing rule). *Smallest, fully chain-resident — first target.*
- **GSEA** → fgsea NES sign + p for the G2/M, E2F, apoptosis, p53 Hallmark sets → bridge → the DDR-signature support now in `mech_rule`'s rationale (promote it to a recomputed antecedent). *Chain-resident (LFC vector + Hallmark sets).*
- **D-DIFF** → limma moderated-*t* genome-wide → WRN top-hit ranking → bridge → `TopDifferentialDependency` ⇒ lift `dd_achilles`/`dd_drive`. *Needs the genome-wide matrix; see §9.*

## 9. Inputs: chain-resident now, Oxen for large data (D53 revision)

Decision (this thread): **inputs stay chain-resident Eigon-CBOR** (the substrate default) — *not* D53's `PinnedExternalFile`-by-URL. This skips D53's "one genuine gap" entirely for xenograft + GSEA, whose inputs are small/moderate.

D-DIFF is the exception: "WRN is the genome-wide top hit" needs the full CRISPR + DRIVE matrices (~9M + ~3M values), too large to inline. The resolution (separate design note, a **D53 revision**, to land *before* D-DIFF): a content-addressed large-data store — **Oxen** (`github.com/Oxen-AI/Oxen`) — behind `PinnedExternalFile.reference` (an `oxen://repo@commit/path` scheme), at the ingestion seam, **kernel-unaware**. Eigenius computes its own `content_hash` over the materialized bytes (trust root independent of Oxen's internal addressing); Oxen joins the *availability* TCB, not the *correctness* TCB. This reconciles "avoid fragile external-file dependencies" with "can't inline genome-scale data": a content-addressed, versioned dataset reference is a third category — as trustworthy as an inlined hash, without bloating the chain. Not required for xenograft/GSEA; gates D-DIFF.

## 10. Phased build plan

- **P1 — worker.** `eigenius-r-worker` cdylib (clone `eigenius-lean-worker`: UDS+CBOR+codec reuse, C ABI, cross-check) + `EigeniusRWorker.R` driver + the matrix/data.frame marshalling. Target: a UDS round-trip + health-check test (`uds_round_trip` analogue), `LocalSpawner` host-`Rscript`.
- **P2 — runtime.** `eigenius-r` `LanguageRuntime` impl + `conventions.rs` + `dockerfile.rs`; `RunRuntimeScript` end-to-end under `LocalSpawner`. Register with the dispatcher.
- **P3 — image.** `RImagePlan` + `renv.lock` + `buildah` build → `ImageDigest`; `DockerSpawner` path + cross-check; an image-build integration test.
- **P4 — mirror.** `eigenius-r-runtime::mirror_gen` (S4 emission) + `RPackageMirror` + `call_method` typed path.
- **P5 — WRN xenograft** (lme4) → `concl_vivo` recomputed (chain-resident).
- **P6 — WRN GSEA** (fgsea, pinned seed) → `mech_rule` DDR-signature antecedent recomputed (chain-resident).
- **P7 — Oxen (D53 revision) + WRN D-DIFF** (limma) → `dd_achilles`/`dd_drive` recomputed.

P1–P4 are the reusable R runtime (the bulk of the work, but a well-trodden clone of two existing runtimes). P5–P7 are the WRN payoffs. Each phase is independently testable and lands behind its own tests, consistent with the prior increments.

## 11. Trust model & caveats

- **Correctness pin = content hashes** (script IRI, input `content_hash`, output hash, `image_digest`). Verification = re-run script S on inputs I in image D, check output hash H. Deterministic for limma/lme4; for fgsea, deterministic only with the pinned seed (§5).
- **Availability TCB** = the image registry + (for D-DIFF) Oxen. Losing them costs reproducibility convenience, not verifiability — any byte-identical copy re-verifies against the chain's hashes.
- **Verification now depends on an external interpreter** (R in the pinned image), unlike the pure-Rust institutions (verify = re-run deterministic Rust in-process). This is the deliberate trade for faithful-by-construction wrapping, and is the same trade Julia/Lean already make. The `RuntimeInvocation` + `ImageDigest` + cross-check are what give "re-run and get the same bytes" teeth.
