# Lean 4 institution tutorial

Slow-walk worked example of the platform's first verification institution. Exercises the full audit-chain shape from [D28 §5.7](../../../design/d28-lean-4-as-institution.md) end-to-end against a real Lean 4 proof: from the chain-committed `Verdict::Holds` back through the proof term, mirror anchor, and chain-side ontology class the proof discharges a claim about.

Read this if you want to know what's actually happening when the [lean-verification notebook](../../../../notebooks/examples/lean-verification.json) runs, how to author your own proofs to land them on the chain, or how the in-process verification path stays tight (no Docker, no IPC, verdict as a direct function call).

## Why Lean is different from the Julia institutions

The five [Julia institutions](../julia-institutions/) follow the same shape: orchestrator-spawned worker containers hosting a Julia ecosystem, communicating with the kernel via Eigon-CBOR over UDS. The runtime substrate's `mirror create → env build → env create → institution install` lifecycle drives every one.

Lean 4 splits this:

- **Authoring side** (substrate-hosted, just like Julia): writing Lean source, running `lake build`, exporting via `lean4export`. Lives in a `LeanEnvironment` Docker image with the toolchain pinned by digest. The substrate's `LeanLanguageRuntime` ([`crates/eigenius-lean-runtime/`](../../../../crates/eigenius-lean-runtime/)) drives image builds and worker dispatch.
- **Verification side** (in-process inside the kernel binary): re-checking `lean4export` bytes via [`nanoda_lib`](https://github.com/ammkrn/nanoda_lib). Lives in [`crates/eigenius-lean/`](../../../../crates/eigenius-lean/), linked directly into the kernel's `cli` crate. No IPC, no orchestrator round-trip.

The split is load-bearing for trust ([D28 §2.3](../../../design/d28-lean-4-as-institution.md#23-substrate-factoring-hosted-authoring-vs-in-process-verification)). The authoring side runs arbitrary user-provided Lean code in a container; the verification side runs `nanoda_lib`'s ~3 kLoC of audited Rust against the exported term. The verdict the kernel publishes is a direct function call inside the kernel process — no chance of a compromised worker forging a `Holds`.

This means a Lean institution is registered with `runtime: in_process` rather than `runtime: external`. The `InProcess` runtime kind ([`kernel/src/institution/registry.rs`](../../../../kernel/src/institution/registry.rs)) lands in Phase 20a; future verification institutions (Rocq, Isabelle/HOL, SMT checkers) will follow the same factoring.

## The five chain shapes

Verification leaves a typed audit trail with five resource kinds:

| Resource | Role |
|---|---|
| [`LeanProject`](../../../design/d28-lean-4-as-institution.md#31-resource-classes) | A Lake project — `lakefile.lean` + `lean-toolchain` + source files. The substrate's authoring runtime builds it and runs `lean4export`. |
| [`LeanEnvironment`](../../../design/d28-lean-4-as-institution.md#71-leanenvironment-extends-runtimeenvironment) | Digest-pinned Docker image + axiom allowlist + lockfile hash. Subclass of `RuntimeEnvironment`. The lockfile-hash field anchors the proof against a specific dependency-tree state. |
| [`LeanPackageMirror`](../../../design/d30-eigon-to-lean-faithful-translation.md) | The generated EigeniusFFI Lake package that mirrors a chain layer's classes into Lean structures. The proof's proposition references types in this mirror; correspondence checking walks these. |
| `LeanProofPayload` | Verbatim `lean4export` output as a string. The exact bytes nanoda re-checks; no compression, no canonicalisation. |
| `LeanProofTerm` | Wires payload + mirror + claim together. Carries the chain-mirrored proposition ([`lean:LeanExpr`](../../../design/d40-chain-mirrored-lean-expressions.md)) and the IRI of the chain-side claim the proof discharges. |

AutoOnLoad fires on `LeanProofTerm` commits and produces a sixth shape:

| Resource | Role |
|---|---|
| `Verdict` (`ctor_name: "Holds" / "Fails" / "Undecidable"`) | The institution's outcome. Committed alongside a `RuntimeInvocation` provenance record. Queryable like any other chain resource. |

## The three-part correspondence check

[D28 §5.5](../../../design/d28-lean-4-as-institution.md#55-the-correspondence-check) specifies what `qc_proof_check` verifies for every `LeanProofTerm` that commits. AutoOnLoad fires it; the kernel rejects the commit if it fails.

1. **Proof validity.** `nanoda_lib::check_proof(payload_bytes)` parses the export JSON, type-checks each declaration against the axiom allowlist on the `LeanEnvironment`, and confirms the named theorem's term has the named type. Implementation: [`crates/eigenius-lean/src/checker.rs`](../../../../crates/eigenius-lean/src/checker.rs). Panic-catching wraps the call so a bug in the bundled checker becomes `Verdict::Fails` with a structured diagnostic, not a kernel crash.

2. **Mirror correspondence.** Walks the proof term's proposition (chain-mirrored as `lean:LeanExpr`) for `Const` references with names like `EigeniusFFI.Patient`. Maps each back to a chain class IRI via the `LeanPackageMirror`'s `mirrored_classes` list. The chain claim's `is_a[0]` must appear in that list. Implementation: [`crates/eigenius-lean/src/institution.rs`](../../../../crates/eigenius-lean/src/institution.rs)'s `structural_correspondence_matches`.

3. **Anchor consistency.** Recomputes the SHA-256 over the mirror's embedded `library_content` archive (using the D30 §10.2 length-prefixed framing) and compares to the declared `library_content_hash`. Mismatch = tampering, even if the proof type-checks.

All three must pass for `Verdict::Holds`. Any one failing produces `Verdict::Fails` with a typed diagnostic (`AxiomNotPermitted`, `PropositionMismatch`, `FFIVersionMismatch`, etc.; see [D28 §9.1](../../../design/d28-lean-4-as-institution.md#91-failure-modes-and-diagnostics)).

## Walking the audit chain (lean-verification notebook)

The capstone notebook ([`notebooks/examples/lean-verification.json`](../../../../notebooks/examples/lean-verification.json)) walks the cycle that closes between the verdict and the ontology class the proof is *about*. Read forward, the chain is:

```text
Patient class                              [Class — the chain-side ontology root]
  ↑ is_a
patient_1                                  [Patient — the Eigon claim instance]
  ↑ claim_iri
proof_term                                 [LeanProofTerm — proof + proposition + anchors]
  │ ↑ proof_payload
  │  proof_payload                         [LeanProofPayload — verbatim lean4export bytes]
  │ ↑ mirror_iri
  │  mirror                                [LeanPackageMirror — audit anchor]
  │  │ ↑ source_layer
  │  │  bootstrap head layer               [universal ancestor of every claim layer]
  │  │ ↑ library_content_hash
  │  │  sha256:af662f…d6510e               [tamper-detection anchor]
  │  │ ↑ library_content.files[]
  │  │  Capstone.lean, EigeniusFFI.lean,   [the Lake project that produced the proof]
  │  │  lakefile.lean, lean-toolchain
  │  ↑ mirrored_classes
  │   Patient                              [back to the ontology root — cycle closes]
  ↑ verdict_subject
Verdict("Holds")                           [the verdict — D28 §5.7 closure]
```

The notebook's four EigenQL cells trace one backward edge each:

1. **Find the verdict.** `MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:institution:verdict_subject": ?subj, "urn:eigenius:core:ctor_name": ?c } WHERE ?subj = "urn:eigenius:demo:lean:proof_term" RETURN [] { ... }` — one row, `ctor = "Holds"`. The IRI is the audit anchor for everything else.
2. **Find the proof term.** Match on `urn:eigenius:lean:LeanProofTerm`, project `target_name`, `claim_iri`, `mirror_iri`. Three cross-references that pin the proof to its chain-side counterparts.
3. **Find the mirror.** Match on `urn:eigenius:runtime:RuntimePackageMirror`, project `source_layer`, `library_content_hash`, `mirrored_classes`. The `source_layer` is the IRI form `urn:eigenius:layer:<hex>` of the chain layer the mirrored classes were authored in.
4. **Find the Patient class.** Match on `urn:eigenius:core:Class`, project `short_name`. Closes the loop — `short_name = "Patient"` is the string nanoda's printer would render `EigeniusFFI.Patient`'s `Const` reference as, and the structural correspondence check pinned this mapping.

Every byte that went into the verification — Lake project sources, the toolchain pin, the verbatim `lean4export` JSON, the chain-side class declaration, the source layer the mirror anchors to — sits on the chain as a typed, queryable, content-addressed resource. Nothing in the chain is *trusted*; everything is *anchored*.

## Authoring your own proof

This is the high-level shape; the demo's generator binary ([`crates/eigenius-lean/examples/gen_verification_demo.rs`](../../../../crates/eigenius-lean/examples/gen_verification_demo.rs)) is the reference implementation.

1. **Author the chain side.** Commit the class declaration and the instance the proof is *about* via standard ESL.

   ```esl
   namespace core = "urn:eigenius:core";
   namespace ex = "urn:example";

   class ex:Patient {
       description = "...";
   }

   resource ex:patient_1 : ex:Patient { }
   ```

2. **Author the Lake project.** A standalone Lake package that imports `EigeniusLeanCommon` (the hand-authored helper package the substrate ships) and defines the structures the proof refers to under `namespace EigeniusFFI`. The capstone proof at [`lean/research/capstone-proof/`](../../../../lean/research/capstone-proof/) is the canonical reference shape.

   ```lean
   -- EigeniusFFI.lean
   import EigeniusLeanCommon
   namespace EigeniusFFI

   structure Patient where
     weight : Float
     deriving Repr

   end EigeniusFFI
   ```

   ```lean
   -- Capstone.lean
   import EigeniusFFI

   theorem patient_weight_nonneg :
       ∀ p : EigeniusFFI.Patient, p.weight ≥ 0 → p.weight + 10 ≥ 10 := by
     intro p _; omega
   ```

3. **Build + export.** Inside the Lake project:

   ```sh
   lake build
   lake exe lean4export Capstone -- patient_weight_nonneg > proof.json
   ```

   The `proof.json` is what becomes `LeanProofPayload.payload_bytes`. It's verbatim machine output; we don't post-process it.

4. **Compute the library content hash.** SHA-256 over the four files (`lakefile.lean`, `lean-toolchain`, `EigeniusFFI.lean`, `Capstone.lean`) under D30 §10.2's length-prefixed framing. Implementation: the `library_content_hash` helper that ships in [`crates/eigenius-lean-runtime/src/mirror_gen/module_assembler.rs`](../../../../crates/eigenius-lean-runtime/src/mirror_gen/module_assembler.rs).

5. **Decode the proposition.** `bytes_to_lean_expr(proof_bytes, theorem_name)` returns the proposition as a `Value::Json` carrying a `lean:LeanExpr` value (D40 §3.3). The kernel's validator type-checks this against the LeanExpr ontology at commit time; the institution's structural correspondence check walks it for `Const` references.

6. **Commit the five resources** (Patient class, instance, mirror, payload, proof term) as a single Eigon-JSON document loaded via `eigenius load`. AutoOnLoad fires automatically on the proof term's commit and produces the verdict resource.

The generator binary at `crates/eigenius-lean/examples/gen_verification_demo.rs` performs steps 2-6 for the capstone proof and writes the result to `notebooks/examples/lean-verification-demo.eigon.json`. Use it as the executable spec.

## Axiom allowlist policy

Every `LeanEnvironment` declares a `lean_permitted_axioms` list (D28 §7.1). The institution's default — and the value baked into the demo — is the four community-baseline axioms: `propext`, `Classical.choice`, `Quot.sound`, `Lean.trustCompiler`.

`Classical.choice` stays in the default because even trivial proofs through modern Lean stdlib pull it via `Subtype`'s projection helpers — an empty allowlist would reject essentially every real proof. Constructive-only deployments override per-env by stamping a tighter `lean_permitted_axioms` on their own `LeanEnvironment` resource. The override is the audited resource property, not a code patch (D28 §12 — resolved at end of Phase 20a).

## Toolchain pin and upgrade

The pinned Lean toolchain lives at [`lean/runtime-worker/lean-toolchain`](../../../../lean/runtime-worker/lean-toolchain) — single source of truth. Three sibling pins (`lean/common/EigeniusLeanCommon/lean-toolchain`, `lean/research/capstone-proof/lean-toolchain`, `lean/runtime-worker/vendor/lean4export/lean-toolchain`) must match.

Upgrades are manual, not automatic — the substrate's image-digest model makes every toolchain change a new content-addressed `LeanEnvironment`, so existing verified proofs stay valid against their original env digest. Auto-bump would fragment the env-digest space silently. The full upgrade checklist is at [`docs/notes/lean-toolchain-upgrade.md`](../../../notes/lean-toolchain-upgrade.md); follow it whenever bumping.

## Lean toolchain prerequisite

The substrate's Lake-driven worker (`lean/runtime-worker/`) depends on a Rust cdylib (`libeigenius_lean_worker.so`) built from [`crates/eigenius-lean-worker/`](../../../../crates/eigenius-lean-worker/). The crate is a regular workspace member, but its `build.rs` compiles a C bridge against Lean's `lean.h` to talk to the Lean runtime — `lean.h` ships with the Lean toolchain via elan, not with system packages, so **any host that builds the workspace needs elan + the pinned toolchain installed**.

Install elan (one-time) per the standard Lean instructions:

```sh
curl https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh -sSf | sh
```

The pinned toolchain version lives at [`lean/runtime-worker/lean-toolchain`](../../../../lean/runtime-worker/lean-toolchain) (single source of truth — same file the Dockerfile composer bakes into env images). Elan will install it automatically the first time anything reads that file, or you can pre-install:

```sh
elan toolchain install $(cat lean/runtime-worker/lean-toolchain)
```

The build.rs ([`crates/eigenius-lean-worker/build.rs`](../../../../crates/eigenius-lean-worker/build.rs)) discovers `lean.h` via, in order: `EIGENIUS_LEAN_INCLUDE_DIR` env var (explicit override), `LEAN_SYSROOT` env var (set by Lake during builds), or `lean --print-prefix` from PATH (the typical dev-host path).

CI installs elan in the workflow at [`.github/workflows/ci.yml`](../../../../.github/workflows/ci.yml) so `cargo clippy --workspace --all-targets` and `cargo test --workspace` cleanly build everything including the worker cdylib.

The full e2e test ([`crates/eigenius-lean-runtime/tests/lean_image_build_e2e.rs`](../../../../crates/eigenius-lean-runtime/tests/lean_image_build_e2e.rs)) is `#[ignore]` because it also needs Docker + buildah for the env-image step; run with `cargo test -p eigenius-lean-runtime --test lean_image_build_e2e -- --ignored` when verifying changes to the substrate's image-build pipeline.

## Performance shape

A single-theorem proof of the capstone shape (one `omega` against `Float` ordering, transitively pulling in `Subtype` / `Classical.choice` / `LE.le.toLT` and the EigeniusFFI.Patient structure) exports to ~9 kLoC of JSON (~200 KB). `nanoda_lib` re-checks it in under a second on a modern laptop. The integration test [`crates/eigenius-lean/tests/capstone_test.rs`](../../../../crates/eigenius-lean/tests/capstone_test.rs) is `#[ignore]` because of that cost; run with `-- --ignored` when verifying changes touching the institution.

Memory note: nanoda parses the full export into an `Expr`-tree before type-checking. For small proofs this is fast; for Mathlib-scale proofs the parsed tree can be hundreds of MB. The Phase 20b plan ([implementation-plan.md §Phase 20b](../../../design/implementation-plan.md)) sizes this up when a Mathlib-dependent consumer appears.

## Troubleshooting

- **`Verdict::Fails` with `PropositionMismatch`** — the proposition decoded from the proof bytes doesn't match the chain-mirrored proposition on the `LeanProofTerm`. Almost always means the fixture was regenerated against different proof bytes; re-run the generator binary.
- **`Verdict::Fails` with `FFIVersionMismatch`** — the mirror's `source_layer` isn't reachable from the claim's layer, or the proposition references a class the mirror doesn't list. Check that the chain-side class declaration's IRI appears in `mirrored_classes`.
- **`FormatViolation: value '<hex>' does not match format 'urn:eigenius:core:formats:iri'`** — the `source_layer` value isn't IRI-shaped. Wrap the hex layer ID in `urn:eigenius:layer:<hex>`. The institution's check accepts both forms.
- **`MissingRequired` errors at commit** — the `LeanPackageMirror` is missing a required property (`short_name`, `description`, `language`, `generator_identifier`, etc.). The generator binary's `mirror_resource()` function sets all of them; if you're rolling your own, mirror its shape.
- **Verdict missing after kernel restart** — fixed in Phase 20a final round. If recurring, check that the kernel ran a clean shutdown (`docker compose stop kernel` with sufficient grace period). The sync-write durability fix at [`storage/rocksdb/src/lib.rs`](../../../../storage/rocksdb/src/lib.rs) makes branch-ref + layer commits durable across forced restarts.

## Cross-references

- [**Platform §11 — Runtime substrate**](../11-runtime-substrate.md) — the conceptual chapter the authoring side runs on top of.
- [**The formula language guide**](../../formula/README.md) — sibling chain-mirroring story for FormulaTerm (D32). LeanExpr (D40) is the verification-institution counterpart.
- [**ESL §9 — Institutions in ESL**](../../esl/09-institutions.md) — generic D14 institution surface; Lean uses the same dispatch shape as Julia institutions.
- [**D28 — Lean 4 as Verification Institution**](../../../design/d28-lean-4-as-institution.md) — the design spec for the institution itself.
- [**D30 — Eigon → Lean Faithful Translation**](../../../design/d30-eigon-to-lean-faithful-translation.md) — the mirror generator's faithfulness spec.
- [**D40 — Chain-Mirrored Lean Expressions**](../../../design/d40-chain-mirrored-lean-expressions.md) — `lean:LeanExpr` / `lean:LeanLevel` / `lean:LeanName` inductives.
- [**Toolchain upgrade checklist**](../../../notes/lean-toolchain-upgrade.md) — manual upgrade flow with regeneration steps.
- [**implementation-plan.md §Phase 20a**](../../../design/implementation-plan.md) — the phase plan for the institution, including the eight sub-milestones and the capstone test.
