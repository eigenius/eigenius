# Lean toolchain upgrade checklist

## When to use this

Use this checklist any time the pinned Lean toolchain version changes. The single source of truth is [`lean/runtime-worker/lean-toolchain`](../../lean/runtime-worker/lean-toolchain) (today: `leanprover/lean4:v4.29.1`). Every other consumer — the build-script-derived `LEAN_TOOLCHAIN_VERSION` constant, the Dockerfile composer, the EigeniusLeanCommon Lake package, the capstone proof project, the vendored `lean4export` — must end up at the same version. Drift between them produces silent verification mismatches that are very hard to debug after the fact, so the checklist exists to keep the four `lean-toolchain` files in lockstep and to force the regeneration of every derived artifact in one commit.

## Why manual

D28 §12 (resolved at end of Phase 20a) explains the policy: the substrate's image-digest model makes every toolchain change a new content-addressed `LeanEnvironment`, so existing *verified* proofs stay verified against their original env digest. Auto-bump would fragment the env-digest space silently and confuse audit chains. Lean is not a fast-moving security-critical runtime, so the slower-but-explicit cadence is the right tradeoff.

## Steps

1. **Edit the single source of truth.** Bump the version in [`lean/runtime-worker/lean-toolchain`](../../lean/runtime-worker/lean-toolchain) to the new tag (e.g. `leanprover/lean4:v4.30.0`).

2. **Mirror the bump into the three sibling pins.** Same string in:
   - [`lean/common/EigeniusLeanCommon/lean-toolchain`](../../lean/common/EigeniusLeanCommon/lean-toolchain)
   - [`lean/research/capstone-proof/lean-toolchain`](../../lean/research/capstone-proof/lean-toolchain)
   - [`lean/runtime-worker/vendor/lean4export/lean-toolchain`](../../lean/runtime-worker/vendor/lean4export/lean-toolchain)

   Confirm all four are byte-identical (`diff` them) before continuing.

3. **Bump the nanoda_lib dep if the new Lean version requires it.** Check [`crates/eigenius-lean/Cargo.toml`](../../crates/eigenius-lean/Cargo.toml) for the nanoda_lib version; nanoda's parser is keyed against `lean4export`'s JSON schema, which can change across Lean versions. If nanoda hasn't published a compatible release, the upgrade blocks until it does — do not patch around it locally.

4. **Rebuild the workspace.** `cargo build --workspace` — the `eigenius-lean-runtime` build script re-reads `lean-toolchain` and stamps the new `EIGENIUS_LEAN_TOOLCHAIN_VERSION` constant. Confirm `cargo check` is clean.

5. **Regenerate the mirror goldens.** The Lean toolchain version appears in the generated `lean-toolchain` file inside the mirror package, which is hashed into the `library_content_hash` and downstream into the mirror's IRI. So a toolchain bump deterministically changes the mirror digest. Regenerate with:

   ```
   EIGENIUS_UPDATE_GOLDEN=1 cargo test -p eigenius-lean-runtime --test mirror_golden_test
   ```

   Review the diff to confirm only the toolchain line and the `mirror_resource_id` changed; if anything else moved, that's a bug, not an expected upgrade artifact.

6. **Re-run the workspace unit + non-Docker integration tests.** `cargo test --workspace` — must stay green.

7. **Re-run the in-image lake-build Docker e2e.** Cold build is 5–15 minutes; budget for it.

   ```
   cargo test -p eigenius-lean-runtime --test lean_image_build_e2e -- --ignored
   ```

   This builds the full env image with the new toolchain via `elan toolchain install`, stages EigeniusLeanCommon and a synthetic mirror, sed-rewrites the lakefile, and confirms `lake build` produces the expected `.olean` files in-image. Failure here typically means either elan rejected the toolchain tag (typo in step 1) or Lake's library-discovery behaviour shifted across versions.

8. **Re-run the capstone proof end-to-end.** This is the load-bearing audit-chain test — it rebuilds the capstone Lean project against the new toolchain, runs `lean4export`, and confirms the resulting proof bytes re-check through nanoda.

   ```
   cargo test -p eigenius-lean --test capstone_test -- --ignored
   ```

   If this fails, the new Lean version's tactics, stdlib, or `lean4export` shape has shifted in a way that breaks the proof we wrote. Fix the proof, do **not** patch around the failure.

9. **Update `LeanPackageMirror` resources committed downstream.** Any chain that holds a `LeanPackageMirror` resource generated under the old toolchain still references the old library content hash. New mirrors generated post-upgrade get a new IRI per [D30 §10.3](../design/d30-eigon-to-lean-faithful-translation.md), so the old and new mirrors coexist by design — old proofs continue to re-check against their original mirror, new proofs anchor against the new mirror. **No backfill is required**, and no backfill should be performed; the divergence is the audit chain working as designed.

10. **Commit everything in one logical commit.** `lean-toolchain` files (×4) + regenerated mirror goldens + any nanoda_lib bump + any capstone-proof source edits. The commit message should call out the version transition (e.g. `lean: bump toolchain to v4.30.0`) so `git log -- lean/` surfaces the upgrade history cleanly.

## What not to do

- **Do not bump only `lean/runtime-worker/lean-toolchain`** without mirroring into the three siblings. The build-script-derived constant will diverge from what elan installs in the dependent Lake projects, and you'll get "no such toolchain" errors at random points in the test suite.

- **Do not commit a bump without rerunning the capstone test.** A passing `cargo test --workspace` is necessary but not sufficient — the workspace tests don't include the capstone (it's `--ignored` because it parses ~9 kLoC of `lean4export` output through nanoda). The capstone is the only test that exercises the full audit-chain shape against a real Lean proof, so silent breakage there silently breaks Phase 20a's load-bearing claim.

- **Do not backfill old `LeanPackageMirror` resources** to point at the new mirror. The mirror IRI is content-addressed on purpose so the chain records *which Lean version this proof was verified under*. Backfilling erases that history.

- **Do not auto-bump via CI.** A bump is a deliberate event, not a maintenance task. The chain's audit posture depends on every toolchain transition appearing as a discrete, reviewed commit.

## Related

- D28 §12 (resolved at end of Phase 20a) — the policy rationale.
- D30 §10.2 — how the toolchain version flows into `library_content_hash`.
- [`crates/eigenius-lean-runtime/build.rs`](../../crates/eigenius-lean-runtime/build.rs) — the build script that propagates the pinned version into Rust source.
