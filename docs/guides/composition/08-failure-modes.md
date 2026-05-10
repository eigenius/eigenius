# 8. Failure modes across compositions

> **STATUS:** outline only. To be filled in.

## What this chapter covers

Multi-institution flows have failure modes single-institution flows
don't. This chapter is the survival guide. Five categories:

1. **Validation cascade failures.** Chain validation walks nested
   resources; a comorphism's reinserted output is validated against
   the target class's `requires` and any constraints. A failure deep
   in the cascade can be hard to read; the chapter shows how to
   trace from a `Load failed` message back to the offending property
   in the offending sub-resource.

2. **Comorphism dispatch failures.** What happens when the source
   institution's `extract_typed` returns an error, when the
   transformation Component panics, when the target institution's
   `reify` rejects the payload. Each step's failure has a different
   shape; the chapter walks through diagnosing each.

3. **Chain-state races.** AutoOnLoad gates fire synchronously per
   commit, but a multi-cell notebook (or a multi-call program) can
   queue commits faster than gates complete. What "stale Verdict"
   means and when downstream queries can see one.

4. **Provenance gaps.** When a comorphism's chain-reinserted output
   is missing its `RuntimeInvocation` (e.g. because the source
   institution is in-process / WASM and doesn't produce one), what
   downstream auditing can and can't reconstruct. When to require
   external runtimes for verifiable composition.

5. **Cross-host coordination.** WASM-hosted institutions and
   substrate-hosted institutions can compose, but they have
   different failure modes (Wasmtime traps vs. UDS disconnects,
   fuel exhaustion vs. container restart). The chapter calls out
   which symptoms correspond to which host kind.

## Section outline

- **§8.1.** Validation cascade failures
- **§8.2.** Comorphism dispatch failures (extract / transform / reify)
- **§8.3.** Chain-state races and stale Verdicts
- **§8.4.** Provenance gaps under mixed hosting
- **§8.5.** Cross-host failure mode classification table

## Cross-references

- [Formula guide §7](../formula/07-failure-modes.md) — failure modes
  specific to FormulaTerm payloads (the corresponding per-payload
  guide; this chapter is the per-composition view)
- [Platform §13 — Troubleshooting](../platform/13-troubleshooting.md)
  — operational failures of the platform itself

---

Next: **[9. Appendix →](09-appendix.md)**
