# D87 — The verification judgement

*Status: **proposed** `2026-09-04` · design note.*

*Replaces the artifact half of eigenius#160. Companions: the paper
[Judgements, Warrants, and Logics](judgements-and-warrants.tex),
[D74](d74-eigentt-to-lean-externalization.md) (what externalization can and cannot express), and
`docs/notes/judgements-warrants-build-plan.md` §"Open after P7".*

---

## 1. The defect

#160 made a checked Lean proof reach the *verified* grade by emitting a `prov:VerificationTrace`.
`witness_index::emit_from_trace` follows the trace's `prov:resource` to the claim, hashes the
claim's `reflection:canonical_proposition`, and admits `IsVerifiedAs`. Nothing re-checks anything;
the kernel takes the trace's word that nanoda ran.

`eigentt:Judgement`'s own description names this failure mode exactly:

> the reification of justification logic's proof-checker operator — LP's positive-introspection
> axiom `t:F -> !t:(t:F)` names the evidence `!t` that `t` proves `F`, which is what a checker
> returns and what this constructor persists. **An algebra carrying application and sum but no `!`
> runs the checker at commit time and discards the result; a judgement keeps it.**

The trace runs nanoda at commit and keeps a note that it ran. What survives is:

| `prov:proof_system` | `"lean4"` — a string that binds to no checker |
| `prov:proof_term` | an IRI naming the `LeanProofPayload` |
| `prov:resource` | the claim |
| `prov:timestamp` | when |

The externalized proposition the institution compared by `def_eq` is discarded, and no later reader
can re-run the comparison from what is committed.

**Why this matters more than tidiness.** `witness:IsVerifiedAs` has **zero constructors**, so no
term inhabits it and `layer_admits_witness` is the only way one comes into existence — which puts
that function in the TCB, and a wrong admission cannot be caught downstream because an axiom has no
proof to re-check. The witness is postulated on the strength of a note.

## 2. The shape that keeps the result

`eigentt:Judgement` has one constructor:

```
holds(logic : eigentt:Logic, term : eigentt:Term, type : eigentt:Term)
```

*"A CHECKED triple: a checker for `logic` verified `term` against `type`."*

Two `eigentt:Logic` values are declared, and the second is named for this:

| `eigentt:logic_kernel` | the kernel's own type checker |
| `eigentt:logic_lean4` | *"Lean 4, re-checked in process by the `nanoda_lib` kernel reimplementation"* |

The route is already implemented for one resource class. `emit_from_reasoning_sentence`
(`witness_index.rs:291`) reads `justification:proof` off a `justification:Conclusion`, decodes the
judgement, **refuses it when the type is a `Certificate`** — a certificate judgement establishes
nothing about the proposition — and keys `Verified` off the proof's own type. That is a
judgement-backed `Verified`, checked rather than postulated.

Nothing populates `justification:proof`. Both `Verified` routes exist; the trusted one is populated
and the checked one is empty.

## 3. The constraint that shapes the answer

`holds`'s `term` argument is typed `eigentt:Term` — a structural term, not a reference. Two things
follow, and the second was got wrong in discussion before it was checked:

1. **A Lean proof term is not representable.** D74 externalizes propositions (types) in one
   direction, EigenTT → Lean. There is no inverse, deliberately (#159), and `Lam` is refused
   outright (D74 §4.4: Mini-TT lambdas carry no domain). `fun _ h => h` has no EigenTT form.
2. **A `ConstRef` to the payload resource does not resolve.** `resolve_const_ref`
   (`eigentt_type_mirror.rs:1201`) dispatches on the target's class — `core:Class` → `EigonClass`,
   `eigentt:Axiom` → `EigonAxiom`, `core:InductiveType` → `Const`, the five primitives
   short-circuit — and an unresolved `ConstRef` is a `TermMalformed` rejection
   (`eigentt_value.rs:619`). A `lean:LeanProofPayload` **instance** is none of those.

So `holds(logic_lean4, t, P)` cannot name the proof blob directly, and cannot carry the proof
structurally. `t` needs a decision.

## 4. Proposal: the checked proof is an axiom

Declare the Lean-checked proof as an `eigentt:Axiom` whose `eigentt:axiom_statement` is the claim's
proposition, and let

```
t = ConstRef(<proof axiom IRI>)
```

**It resolves.** `eigentt:Axiom` is one of `resolve_const_ref`'s accepted classes, so the judgement
decodes and type-checks with no change to the eigentt fragment.

**It is what the proof means here.** From EigenTT's side a Lean-checked proof *is* an axiom —
asserted, not constructed in this type theory — and its statement is exactly the proposition the
`def_eq` comparison established. The judgement then records **which checker licensed the
assertion**, which is the fact the trace was trying and failing to carry.

**The pattern is already in use.** The demo fixture's `urn:eigenius:demo:lean:Healthy` is an
`eigentt:Axiom` carrying an `eigentt:axiom_statement`, and D74 §4 translates `EigonAxiom` as a
`Const`.

**Re-checking is then defined.** `holds(logic_lean4, ConstRef(a), P)` is re-checkable by: resolve
`a`, read the `LeanProofPayload` it is anchored to, externalize `P`, and run `check_proof` — which
is precisely what `do_proof_check` already does. The judgement stops being a note and becomes a
claim the kernel can re-decide at any time.

The axiom needs an anchor back to the payload so step two is mechanical. That is one property, and
it is the one piece of new vocabulary this note proposes.

## 5. What it changes

| | from | to |
|---|---|---|
| what the institution emits on `Holds` | `prov:VerificationTrace` | the trace **plus** a `holds(logic_lean4, ConstRef(a), P)` judgement, and the axiom `a` |
| how `Verified` is admitted | `emit_from_trace` hashes the claim's proposition | the judgement's own `type` is the proposition — the `emit_from_reasoning_sentence` shape |
| `Certificate.verified` | consumes `witness:IsVerifiedAs(iri, P)` | consumes the judgement |
| `witness:IsVerifiedAs` | postulated by the kernel, zero constructors, in the TCB | removable |

The trace does not go away. It remains the provenance record — *when* the check ran, by what
`proof_system`, against which payload — and the paper's separation holds: the trace is provenance,
the judgement is warrant.

**This is the prerequisite for removing `witness:Is*As`.** `Certificate.verified` cannot lose its
argument until something else inhabits its premise. `Declared` and `Observed` are a separate
question — both plausibly *are* constant specifications over relations the kernel can read at any
time (`declared_by`, the observation relation), which is what
`judgements-warrants-build-plan.md` §"Open after P7" asks. `Verified` is the family where the answer
is no, and this note is why: no relation on the chain lets the kernel recompute "nanoda accepted
this" without re-running nanoda. Making that re-run *possible* is the point.

## 6. Cost

- **Ontology**: one property anchoring the proof axiom to its payload; possibly a slot for the
  judgement on a non-`Conclusion` claim. Bootstrap-resident, so it rides #235's reseed.
- **Institution**: `do_proof_check` already holds all three arguments at the moment it discards
  them — the logic is fixed, `P` is the `Exp` it just compared, and the payload is resolved.
- **Kernel**: reuse `emit_from_reasoning_sentence`'s decode-and-refuse-a-certificate logic; it is
  not specific to `Conclusion` beyond where it reads the slot.

## 7. Open

1. **Where the judgement lives** when the claim is not a `justification:Conclusion`. The demo's
   claim is a `demo:lean:Patient` instance. Either the slot generalises off `Conclusion`, or the
   institution emits a `Conclusion`, or the judgement rides the trace.
2. **Who declares the axiom** — the institution at check time, or the author alongside the proof
   term. Institution-minted keeps authors from asserting an axiom no checker licensed; author-minted
   keeps the kernel from writing chain declarations.
3. **Whether `Verified(iri)` in the justification term names the claim or the axiom.** It names a
   chain resource by IRI, and after this change there are two candidates.
