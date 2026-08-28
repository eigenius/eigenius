# Build plan — Judgements, Warrants, and Logics

**Status: plan.** Written `2026-08-28`. Implements
[`docs/design/judgements-and-warrants.tex`](../design/judgements-and-warrants.tex). The derivation
record is [D82](../design/d82-propositions-witnesses-and-logics.md); the description of what exists
is [D81](../design/d81-the-epistemic-stack.md), whose findings this plan closes.

**Posture.** The paper specifies replacement, not migration. No backwards compatibility is required:
each reseed rewrites the chain. Phases are ordered so that every gate is runnable and every phase
leaves the tree green.

---

## 0. Surface, measured

Counts taken `2026-08-28` against the working tree. They size the work; they are not the design.

| area | surface |
|---|---|
| grade classes (`{Declared,Observed,Derived,Verified}Resource`) | **21 Rust files** with non-test occurrences, **9 ontologies** |
| `eigentt:TypeExpr` | **21 Rust files**, **51 ontology sites** |
| witness machinery | `witness/mod.rs` 251 lines, `layer/witness_index.rs` 521, `program/check_hooks.rs` 101 |
| reasoning crate | `grade.rs` 607, `extract.rs` 492, `project.rs` 383, `validate.rs` 265, `institution.rs` 182 |
| `DerivedEvidence` / `IsDerivedAs` in authored artifacts | **106 occurrences** across `ontologies/`, `demo/`, `experiments/` |
| validation rules | 15 files under `kernel/src/validation/rules/` |

**Two findings change the shape of the work.**

- **Cycle detection exists.** `kernel/src/layer/declaration_order.rs` already computes a topological
  order over the layer's declaration dependency graph and reports `OrderError::Cycle`, and it already
  reasons about `core:mentions`. §5.3's well-foundedness check extends this rather than building it.
- **The grade classes have writers and no structural readers.** D81 §5.1 established that no reader
  grants an entitlement on an epistemic class. The 21 files above are overwhelmingly *writers* and
  doc comments. P5 is therefore a deletion with a computed replacement, not a consumer migration —
  but see P0, which must confirm this rather than inherit it.

**Bootstrap-touching phases force a reseed** (~12 min, per `bootstrap_drift_reseed`): P1, P2, P5, P7.
Batch bootstrap edits within a phase; do not spread one edit across two.

---

## P0 — Measure before building

**No code.** Produces numbers that determine whether P2 and P5 are small changes or data migrations.
Every later phase is sized by this one.

1. **Does the lexicon survive check mode?** Sample `lexicon:cat`, `lexicon:sem_type` and
   `lexicon:term` values across the loaded lexicon; decode each and `check` it against its intended
   type offline. Report the failure rate and the failure shapes. **This is the plan's largest
   unknown**: ~7.6M entries carry these slots, written by three producers
   (`dcg/glossary.rs`, `dcg/augment.rs`, `lexicon-align/emit.rs`), and Rule 21 has never checked that
   a `lexicon:cat` value is a `lexicon:Cat` — it infers a type and discards it.
2. **Confirm the grade classes have no structural readers.** Re-derive D81 §5.1 mechanically: for
   each of the 21 files, classify every non-test occurrence as writer, reader, or comment. A single
   genuine reader changes P5 from deletion to migration.
3. **Inventory the persisted chain**, not just the tree: how many `DerivedEvidence` leaves,
   `IsDerivedAs` witnesses and grade-class stamps exist on the current chain. Sizes the reseed and
   the P4 invalidation.
4. **Establish the baseline.** Run the parse gate (`--release`, per `parse_sweep_must_be_release`)
   and the WRN demo end to end on the current branch, and record the numbers. Nothing later may
   regress against this baseline, and a regression must be attributable to a phase.

**Exit:** a numbers note under `docs/notes/`. No source change.

---

## P1 — `eigentt:Term`, and the `Judgement` inductive

**Bootstrap edit → reseed.**

- Rename `eigentt:TypeExpr` to `eigentt:Term` across `ontologies/` (51 sites) and Rust (21 files).
  The 20 constructors are unchanged; the class was named for the type-level fragment it originally
  carried and has held lambdas, pairs, projections and literals for some time.
- Declare `eigentt:Judgement` as an inductive with one constructor
  `holds(logic, term, type)`, and `eigentt:Logic` with the two inhabitants the system can check.

**No behaviour change.** Nothing yet ranges over `Judgement`; validation is untouched.

**Exit:** reseed clean at the P0 resource count; `cargo test --workspace`, clippy and fmt green;
parse gate and demo unchanged against the P0 baseline.

---

## P2 — Uniform check-mode validation

**Depends on P1. Bootstrap edit → reseed.**

Replace Rule 21's three-step shape (decode, `check_infer`, plus a `PROPOSITION_SLOTS` special case)
with one rule over `Judgement`-ranged slots: decode both fields, check the type is a type, check the
term against it in **check mode**.

Retire, in the same phase:

- `wk::PROPOSITION_SLOTS` — the hardcoded list of slots required to hold propositions;
- the `eigentt:definition_body` exemption (Rule 24's separate check subsumes into the uniform rule);
- the `core:param_kind` / `core:type_name` exemption (Rule 23's telescope-scoped check);
- the `check_infer`-then-discard path for every other `Term`-ranged slot.

**P0's measurement decides the migration.** Slots holding bare lambdas with their type in a
neighbouring field become single `Judgement` values. Slots whose values are already self-describing
may instead require `Ann`, which the kernel's term language already provides and whose typing rule is
exactly this rule (`check_infer(Ann(e,T))` checks `e` against `T`).

**Exit:** the four obligations D81 recorded as declared-but-unchecked — `lexicon:cat` against
`lexicon:Cat`, `lexicon:sem_type`, `eigentt:axiom_statement`, `eigentt:definition_type` — are
enforced, with a test each. No exemption list remains in `eigentt_value.rs`.

---

## P3 — The proof term, and the two-layer fix

**Depends on P2.** The soundness fix; it closes D81 §5.2.

- `reasoning:ReasoningSentence` gains a `Judgement`-ranged slot carrying a term checked against the
  sentence's `proposition`.
- `verification_trace` (`crates/eigenius-reasoning/src/validate.rs`) stops writing the sentence's own
  IRI into `reflection:proof_term`. For `proof_system = kernel`, `proof_term` names a term the kernel
  checked at `t : P`. A `JustifiedBy` certificate has type `JustifiedBy(j, P)`, not `P`, and no rule
  connects them.
- `emit_from_reasoning_sentence` (`kernel/src/layer/witness_index.rs`) stops minting `Verified` from
  `is_a` membership plus a hashable proposition. It keys off the checked judgement.

**Exit gate — write this test first and watch it fail.** Build a `Declared` claim, cite it from a
second sentence with `JustifiedBy.verified`, and assert `is_fully_verified` returns **false**. On the
current kernel it returns true: `DeclaredClaimGrader` writes a sentence justified by
`DeclaredEvidence`, the witness emitter mints `IsVerifiedAs` on it regardless, and the citation path
is the documented one. The test must fail before P3 and pass after.

---

## P4 — Three grounds

**Depends on P3.** Invalidates authored artifacts; batch with P5's reseed if convenient.

| | current | after |
|---|---|---|
| `JustificationTerm` constructors | 7 | 6 — `DerivedEvidence` removed |
| `witness:Is*As` | 4 | 3 — `IsDerivedAs` removed |
| `JustifiedBy` grounding constructors | 4 | 3 |
| `project.rs`'s `Ground` | 4 | 3 |

- Institutions emit a **composite** justification term — `App(Declared(plan), Observed(inputs))` —
  in place of a single opaque leaf. `emit_from_institution_derivation` and the statistics institution
  are the two producers to change.
- Remove the hardcoded `IsVerifiedAs → IsDerivedAs` coercion in `check_layer_with_coercion`. It
  implements a lattice the paper rejects, and it is not driven by the ontology's `subclass_of`.
- Repair the 106 authored occurrences in `ontologies/`, `demo/` and `experiments/`.

**Exit:** for a statistics-derived claim, `leaves_of(term, Observed)` returns the sample set and
`survives_without(dataset)` returns false. Both answer wrongly today, in the reassuring direction.

---

## P5 — Provenance and warrant as independent axes

**The largest phase. Bootstrap edit → reseed.** Depends on P4 for the grounds vocabulary.

- **Delete the grade classes** `{Declared,Observed,Derived,Verified}Resource` and
  `reflection:epistemic_status`. The latter lets a trace nominate the grade of its own output, which
  is the self-nomination the design forbids; its zero readers are correct and the declaration is the
  defect.
- **Break `VerifiedResource subclass_of DerivedResource`.** Verified is not a special case of
  derived. The relation carries no `requires` inheritance in practice — `DerivedResource` requires
  nothing — and nothing constrains a property to `DerivedResource` via `allows_only` or
  `class_types`, so the removal is free.
- **Provenance becomes relations**, mapped to PROV: `wasAttributedTo`, `wasGeneratedBy`, `used`,
  `hadPlan`, `hadPrimarySource`. The existing `declared_by` / `source` / `derivation` properties are
  the starting points.
- **Warrant becomes a query** over the justification term. Nothing stores it. Index it if the cost
  requires; an index is a cache rebuildable from the relations, which a stamp is not.
- Update the 21 Rust writers and 9 ontologies accordingly.

**Exit:** no resource carries a stored epistemic grade; provenance and warrant are answerable as
queries; `notebooks/examples/stats-and-reasoning.json` — the one consumer found filtering on a grade
class — is updated and runs.

---

## P6 — Well-foundedness

**Depends on P4.** Independent of P5.

Reject at commit any justification whose premise's support transitively includes the premise. Extend
`declaration_order.rs`'s existing topological pass rather than adding a second graph walk: it already
detects cycles over the declaration dependency graph and already reasons about `core:mentions`, which
projects term references into the triple index.

The condition is vacuous on `Declared` premises, which have no support to inspect. That carve-out is
required, not convenient: constant specifications may be self-referential, and self-referentiality is
unavoidable for realising some S4 theorems in LP.

**Exit:** a test constructing the two-layer retroactive-upgrade cycle from the paper's §5.3 and
asserting the commit is rejected.

---

## P7 — Relocate what the kernel owns; state the boundary operationally

**Bootstrap edit → reseed.** Depends on P3 and P4.

**The kernel owns what it must construct; the chain owns what the kernel only has to check.** That
line places the witness types inside the kernel and leaves the certificate vocabulary outside it.

- **`witness:Is*As` moves to kernel base vocabulary.** `synthesize_chain_witness` produces inhabitants
  of those otherwise-empty types — by constant specification for attributions, and from a committed
  judgement for `Verified` after P3. A type the kernel inhabits cannot be owned by a layer above it.
  This also replaces `check_hooks.rs`'s recognition of witness positions by four hardcoded short
  names, which admits any inductive anywhere carrying one of those names, with IRI resolution.
- **`JustifiedBy` and `JustificationTerm` stay chain-declared.** The kernel verifies a constructor
  application against its declared type; one argument's type is a witness type it recognises, and it
  needs no knowledge that the constructor belongs to `JustifiedBy`. Chain-declared inductives exist
  so the kernel can check terms of types it does not carry, and this is the case they were built for.
  Keeping the algebra in a layer also keeps the system's position in the J / J4 / JT family an
  ontology edit rather than a kernel change.
- **`project.rs`'s support algebra stays** as a query surface over retained terms. P4 changes its
  ground enumeration and nothing else.
- **`ValidateJustification` stops being a dispatched AutoOnLoad query**, absorbed by P2's uniform
  check-mode validation: checking a certificate is type checking, which the kernel does not delegate.
  D81 recorded that `dispatch_auto_on_load_for_layer` has one call site and no test; write that test
  against whatever the check becomes, once.
- **State the protocol operationally.** The question is not whether a participating logic satisfies
  the definition of an institution, but whether the system can hold and re-check a witness for the
  claims it establishes. A logic supplies vocabulary, a decision procedure yielding a verdict,
  derivation resources, and optionally a judgement. It does not assign a warrant, define a witness
  kind, or establish `Verified`. Admitting a new hosted checker requires the two arguments the paper
  names: soundness of its `⊢` against its `⊨`, and satisfaction-preservation by its comorphism.

**One assumption is load-bearing.** P6 is kernel enforcement and must inspect the support relation.
The paper discharges it generically, over `core:mentions` reference edges, so the kernel detects
cycles without distinguishing `App` from `Sum`. If that proves wrong — if the check requires the term
algebra's semantics rather than generic reference edges — the kernel needs the algebra and the
division above moves.

**Exit:** `Verified` is reachable only through a checked judgement; the kernel owns every type it
inhabits; and hosting a checker is documented as adding both obligations, and the checker's
implementation, to the trusted computing base.

---

## Verification, every phase

- `cargo test --workspace`; `cargo fmt --all -- --check`; `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`.
- The parse gate, **`--release`** — a debug build fakes grammar gaps through NbE stack overflow.
  Baseline from P0; grammar-gap 0 and the hit count must not regress.
- The WRN demo end to end. Take the stack down first; staging removes the store directory under a
  live RocksDB otherwise.
- After any bootstrap edit: reseed, and check the resource count against P0.

## Risks

| risk | phase | mitigation |
|---|---|---|
| the lexicon does not survive check mode | P2 | P0 measures it offline first; if the failure rate is material, P2 splits into a repair pass and a rule change |
| a genuine structural reader of a grade class exists | P5 | P0 classifies all 21 files before any deletion |
| reseed count grows | P1, P2, P5, P7 | batch bootstrap edits within a phase; P4 and P5 may share one |
| the projection algebra changes under consumers | P4 | `project.rs` itself is correct and stays; only the `Ground` enumeration and what institutions emit change |
| one-step cycle checking proves insufficient | P6 | the paper already concludes it is; P6 implements transitive closure over `core:mentions` from the start |
