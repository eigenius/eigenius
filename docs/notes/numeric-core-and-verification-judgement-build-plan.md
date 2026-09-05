# Build plan — the numeric core and the verification judgement

*Branch: `numeric-core-and-verification-judgement`. Covers eigenius#235, [D86](../design/d86-the-numeric-primitive-core.md),
[D87](../design/d87-the-verification-judgement.md), and §3.5(b) of the kernel-run-records batch.*

*Follows `kernel-run-records-build-plan.md` (merged as #238) and
`judgements-warrants-build-plan.md`, whose P0–P7 all landed.*

---

## 0. Why these travel together

**One reseed.** Every ontology edit below is bootstrap-resident, so each one alone would cost a
reseed — which invalidates every staged snapshot and forces re-deriving the demo artifacts and both
parse baselines. Paid once, the marginal cost of the second through fifth edit is zero.

**And one argument.** D86 makes a measured quantity *relatable* — `0.0 ≤ x` meaning Lean's `≤`.
D87 makes a checked proof *re-decidable* rather than attested. Both are the same move: shrink what
the platform asserts and enlarge what it can recompute. They share the `prov:VerificationTrace`
slots and the D30-emits-`def`s gap (eigenius#236), so splitting them would mean two reseeds and two
passes over the same code.

## 1. The ontology edit — one pass, then one reseed

All five, in `ontologies/`:

| # | edit | source |
|---|---|---|
| 1 | declare `≤` and `float_ieee_eq` over `core:float` | D86 §3.2, §3.3, §6 |
| 2 | a permitted-axiom slot on `prov:VerificationTrace` | D87 §5 |
| 3 | a checker-identity slot on `prov:VerificationTrace`, as **kind + value** | D87 §9.3 |
| 4 | `witness:IsVerifiedAs`'s description — false since #160 | #235 |
| 5 | delete `justification:VerifiedPropositionView` + `justification:source_verified_resource` | #235, confirmed `2026-09-05` |
| 6 | delete `components:Combine` / `Extract` / `Transform` | kernel-run-records §3.5(b) |

**Nothing here is undecided.** D86 §6 settled the three that were open (`2026-09-05`); #235's edit 4
was confirmed; §3.5(b) was argued in the previous batch. Edit 5 also needs the two `esl::compile`
test references repointed and the `bootstrap/mod.rs:1375` registration removed.

**Sequencing:** land all six, then reseed **once**, then the verification gate (§5). Do not reseed
between edits.

## 2. D86 — the correspondence lives in Rust

§6.1 decided the primitive correspondence is a **table in the generator**, beside the externalizer
that consumes it, not a property on the chain axiom. The reason is a security one and worth
restating at the implementation site: `docs/guides/esl/09-institutions.md` §9.11.2 puts **each
formal comorphism in the TCB**, so an on-chain property would make a TCB entry authorable by
committing a resource — the self-nomination shape eigenius#23 deleted `epistemic_status` for.

So: a small fixed table mapping the two chain relations to `@LE.le.{0} Float instLEFloat` and
`instBEqFloat`, read by `crates/eigenius-lean/src/externalize.rs` when it meets the corresponding
`Const`. Short enough to read in full, per §6.1.

**`float_ieee_eq`, not `float_eq`.** The name carries the IEEE-ness so it cannot be read as
propositional equality by someone who has not read §3.3 — where `Eq` on `Float` is structural and
separates `0.0` from `-0.0`.

**NaN is admitted** (§3.4, decided `2026-09-05`, [Goldberg 1991](https://doi.org/10.1145/103162.103163)).
No mandatory refinement. Nothing in the tree checks NaN today and nothing should start; a claim
`0.0 ≤ x` is simply **false** for a NaN `x`, which is the right answer.

## 3. D87 — the judgement, and what it needs first

### 3.1 The fixture comes first (D87 §6)

`demo:lean:patient_1` is a `Patient` instance carrying a `canonical_proposition` that never mentions
it, and the proof is a tautology (`fun _ h => h`). So the demo shows the plumbing runs, not that it
discriminates. **Nothing downstream can be tested meaningfully until this is fixed**, which is why
it leads.

Regenerate through `gen_verification_demo` so the claim is a `justification:Conclusion` with
`subject_iri`, carrying a proposition that is *about* its subject — plus a near-miss variant that
must fail.

### 3.2 The term former (D87 §4.2)

A distinct `eigentt:Term` former for a checked proof, so the chain can tell "asserted without proof"
from "checked by nanoda". **Not** `eigentt:Axiom`, which is defined as *"a closed term whose type
the kernel admits without checking the term itself"* — the opposite of what is being recorded, and
the conflation #205 and #23 both eliminated elsewhere.

The largest piece: it lands in the D47 codec, conversion, and D74's exhaustive 43-variant match —
which will **refuse** it for externalization, since a checked-proof reference has no Lean
counterpart to translate to.

### 3.3 The emit (D87 §7)

`do_proof_check` emits `holds(logic_lean4, Checked(t), P)` alongside the trace. It already holds all
three arguments at the moment it currently discards them: the logic is fixed, `P` is the `Exp` it
just compared by `def_eq`, and the payload is resolved.

Then `Certificate.verified` consumes the judgement instead of `witness:IsVerifiedAs`. **Measured
`2026-09-04`: zero certificates cite `Certificate.verified` and zero resources carry
`justification:proof`, so this is greenfield** — no migration, no invalidated leaves.

### 3.4 Deploy by digest (D87 §9.3)

`deploy/bicep/modules/kernel.bicep` and `docker-compose.yml` pin by **tag** today, so neither
deployment is reproducible even in principle. Pin by digest and pass `EIGENIUS_IMAGE_DIGEST`; the
identity slot records `image_digest` when present and falls back to `source_pin`.

Worth doing on its own merits, independent of D87.

## 4. Deliberately out

- **eigenius#236** — D30 emitting chain definitions as Lean `def`s. It is the row of D86 §5 that
  carries the design property, and it is a generator rewrite. This batch declares the relations and
  wires the correspondence; #236 is what would make the two sides agree *by construction*.
- **Removing `witness:Is*As` entirely.** D87 §7 makes `IsVerifiedAs` removable; `Declared` and
  `Observed` are a separate question, and `judgements-warrants-build-plan.md` §"Open after P7" asks
  it once for all three. That review is live and is not this batch.
- **An exporter for the PROV mapping** (`docs/spec/w3c-prov-mapping.md` §5) — needs the in-process
  Activity gap closed first (#145 territory).

## 5. Verification

Per `judgements-warrants-build-plan.md` §"Verification, every phase":

- `cargo test --workspace`, `cargo fmt --all -- --check`, `RUSTFLAGS="-D warnings" cargo clippy
  --workspace --all-targets`.
- **Written as failing tests first**: a claim whose proposition is about its subject verifies, and
  its near-miss fails (§3.1); a `Checked(t)` term round-trips the D47 codec and is refused by the
  externalizer (§3.2); a `Holds` emits a judgement whose type is the claim's proposition (§3.3).
- **After the reseed**: the demo artifacts re-derive, and both parse baselines are re-run. Match
  baseline provenance on every axis — including `--umls-all`, whose absence is silent.
- **The count that matters**: after §3.3, a Lean `Holds` produces a judgement *and* a trace, and the
  `Verified` witness keys off the judgement rather than the trace. Measure it.

## 6. Order

1. **§1's six ontology edits**, then the reseed, then §5's baselines. Everything else builds on the
   reseeded chain.
2. **§3.1 the fixture** — nothing downstream is testable before it.
3. **§2 the correspondence table** + the two relation declarations become usable together.
4. **§3.2 the term former**, the largest and riskiest piece.
5. **§3.3 the emit**, which is small once §3.2 exists.
6. **§3.4 deploy by digest**, independent — any time.
