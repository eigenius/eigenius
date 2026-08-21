# D73 — Justification logic: witnesses, traces, and the epistemic lattice

*Status: design memo · `2026-08-21`. No code beyond the three defect fixes already on branches.*

*Arises from [#137](https://github.com/eigenius/eigenius/issues/137), [#175](https://github.com/eigenius/eigenius/issues/175),
[#191](https://github.com/eigenius/eigenius/issues/191) and [#159](https://github.com/eigenius/eigenius/issues/159), which are
four faces of one requirement. Reference: Artemov & Fitting, *Justification Logic*, 2020
(`references/publications/justification-logic-artemov-fitting-2020.txt`).*

*Depends on [D39](d39-justification-logic.md) (the `JustificationTerm` interlingua), D49 (witness emission),
[D6b](d6b-reasoning-trace-schema.md) (the epistemic cluster), [D68](../notes/d68-claim-kinds.md) (the two-axis claim),
[D72](d72-declaration-provenance.md) (agent vs warrant).*

## 0. The claim of this document

`reasoning:JustifiedBy(j, P)` **is** Artemov's `t:F`. That is not an analogy — the constructors are his axioms. The
theory is largely right and largely implemented. What is missing is not the logic but the **discipline around its
edges**: whether the type can be written, whether its indices are respected, whether `P` is a proposition, and — the
one substantive hole — whether a *verified* resource's proof term has anything to do with the proposition it is
recorded against.

## 1. What is already correct

`ontologies/reasoning/reasoning.esl` declares

```
data reasoning:JustifiedBy : reasoning:JustificationTerm -> Prop -> Type 0
```

an **indexed** inductive family, and its constructors are the justification-logic axioms with conclusions the checker
*derives* rather than the author asserts:

| constructor | Artemov |
|---|---|
| `app : JustifiedBy(j1, A -> B) -> JustifiedBy(j2, A) -> JustifiedBy(App(j1,j2), B)` | application, `s:(A→B) → (t:A → [s·t]:B)` (1.1) |
| `sum_l` / `sum_r` | sum `+`, monotonicity of evidence |
| `spec_str` | universal specialization — an extension, not base JL |
| `declared` / `observed` / `derived` / `verified` | the **Constant Specification**: `c:A` admitted from a chain witness |

The four CS constructors each require a witness — `witness:IsDeclaredAs(iri, P)` and siblings — so a constant enters
the specification only when the chain carries the pairing. All four categories are live: `witness_index.rs:172` maps
`ObservationTrace` to `Observed`, and the other three likewise.

This is the shape Artemov argues for over the modal one. §1.1 of the book:

> `K(A → B) → (KA → KB)` … does not specify dependencies between `K(A → B)`, `KA`, and `KB`, [so] the purely modal
> formulation leaves room for a counterexample.

The dependency **is** the content. Everything below is about protecting it.

## 2. Four requirements on the typing of witnesses

A witness is an inhabitant of `JustifiedBy(j, P)`. For that to mean anything:

| # | requirement | status |
|---|---|---|
| **(a)** | the type is **expressible** | **BROKEN** — see §3 |
| **(b)** | different `P` gives a **different type** | fixed, #137 (branch) |
| **(c)** | `P` is a **proposition** | fixed, #175 + #191 (branches) |
| **(d)** | conclusions are **derived by rules**, not asserted | already satisfied — §1 |

(b) and (c) were both live defects until this week, and they interact: measured on branch, #175's gate is defeated by
#191's arm through `Exp::Ann` (which returns its ascription as the inferred type), while #191 alone leaves a chain with
a non-proposition `canonical_proposition` and no citing sentence committing clean. Neither closes the hole alone.

Losing (b) is exactly the collapse Artemov warns about: with indices ignored in conversion, `JustifiedBy(j, P)` and
`JustifiedBy(j, Q)` are the same type, so a certificate for one claim discharges another. The term survives; the link
does not.

## 3. (a) The type cannot be written

`data reasoning:JustifiedBy : reasoning:JustificationTerm -> Prop -> Type 0` gives index #0 the declared type
`EigonClass(JustificationTerm)`, while its inhabitants — `App(...)`, `DeclaredEvidence(...)` — are
`InductiveType(JustificationTerm)` values. Writing the type in ESL therefore fails `check_type` with
`InductiveType(…) ≠ EigonClass(…)`.

It is invisible today only because every node of a `reasoning:certificate` is a **constructor**, and the constructor
path builds the expected type from the declaration instead of checking index arguments against declared index types.

Consequence worth stating plainly: **the one relation carrying the platform's guarantee is the one relation whose type
the surface language cannot express**, which is why the conversion rule protecting it went essentially unexercised.

Fixing this is a prerequisite for testing anything else here at the ESL level.

## 4. What a trace is for, per epistemic class

The epistemic classes and their traces are not symmetric, and the asymmetry is the subject of §5.

| class | trace | what the trace records | required? |
|---|---|---|---|
| `DeclaredResource` | `DeclarationTrace` | **who** asserted it (`declared_by`) + when | `declared_by` required |
| `ObservedResource` | `ObservationTrace` | **where** it came from (`source`) | `source` required |
| `DerivedResource` | `ProgramTrace` | **which program** produced it, from which inputs | `derivation` recommended |
| `VerifiedResource` | `VerificationTrace` | `proof_system`, `proof_term`, `derivation_trace` | both required |

For Declared, Observed and Derived the trace answers a *provenance* question, and the witness pairing
`Is⟨Category⟩As(iri, P)` is an assertion **about the chain**, discharged by the resource existing with its trace. That
is legitimate: the CS is exactly a set of assumed pairings, and Artemov's CS is likewise assumed rather than proved.

Verified is different, and that difference is the gap.

## 5. The Verified gap: a proof term that proves nothing in particular

`reflection:VerificationTrace` requires `resource`, `proof_system`, `proof_term`, `derivation_trace`, `timestamp`.
`proof_term` is documented as *"IRI of the proof term in blob storage"*.

So the trace records **which resource** and **where the proof blob is**. Nothing anywhere relates the *statement the
proof proves* to the resource's `canonical_proposition`. `IsVerifiedAs(iri, P)` is admitted on the trace's say-so.

This is [#159](https://github.com/eigenius/eigenius/issues/159) — "nothing binds a Lean proof to the claim it is
supposed to prove" — and in justification-logic terms it is the sharpest defect in the system: for Declared and
Observed the pairing is an *assumption* and everyone knows it; for Verified the pairing purports to be **checked**, and
is not. `VerifiedEvidence(iri)` is the strongest justification the term algebra offers, and it is currently the least
earned.

What a proper solution requires:

1. **The proof's statement must be recovered, not trusted.** A Lean proof term has a type; that type is the theorem. The
   verification path must extract it rather than accept a blob pointer.
2. **It must be compared to `canonical_proposition` across the comorphism.** The Lean statement lives in Lean's term
   language and the proposition in EigenTT. D30's translation is the bridge, and comparison must happen on one side of
   it with the translation itself trusted or checked — that is a decision, not an implementation detail.
3. **Failure must be a rejected commit**, not a recorded discrepancy. Rule 21 already puts the checker on the commit
   gate; `IsVerifiedAs` should be admitted by the same discipline.

Until then the honest reading of `VerifiedResource` is "someone attached a proof-shaped blob", and the design should
say so rather than implying more.

## 6. Positive introspection is NOT what this needs — and D39 already settled it

An earlier draft of this document argued that Verified wants Artemov's `!` (proof checker),
`t:F → !t:(t:F)`, on the grounds that a machine-checked proof is a warrant whose validity is itself witnessed.
That conflates two different things and is withdrawn.

`!` internalises *"the fact that t justifies F is itself justified"* — a meta-claim about the justification relation.
The §5 gap is not about meta-claims at all: it is that the CS admission for `IsVerifiedAs(iri, P)` **purports to be
checked and is not**. Fixing it needs no new operation; it needs the existing admission to do what it claims.

[D39](d39-justification-logic.md) §11 excludes `!` as an explicit non-goal, with the note that adding it would lift the
system into the `J4`/`LP` family and would require a new `JustificationTerm` constructor plus a matching `JustifiedBy`
rule. That exclusion stands and this document does not reopen it.

**What this document does correct in D39** is one sentence in §10:

> The system is *partially factive*: `VerifiedEvidence`-grounded justifications imply truth (the Lean checker validated
> the proof, so the proposition holds), but the other groundings do not.

The parenthetical is the claim §5 shows is unearned. The Lean checker validated *a* proof; nothing establishes that the
proof's statement is the resource's `canonical_proposition`. So `VerifiedEvidence` is currently no more factive than
`DeclaredEvidence` — it is an assumed pairing wearing the vocabulary of a checked one. D39's factivity claim becomes
true exactly when #159 is closed, and not before.

That asymmetry is worth stating in D39 itself, because the factivity of `VerifiedEvidence` is load-bearing: it is what
lets a chain of inferences preserve the `Verified` category end to end (D39 §10, "inference rules as Declared and
Verified resources").

## 7. Declared resources versus claims

Two lattices meet here and are deliberately not the same one.

**The epistemic axis** (`reflection`) answers *where knowledge came from*: Declared, Observed, Derived, Verified. It
determines which CS constructor can admit a witness.

**The discourse axis** (`encoding`) answers *what kind of assertion this is*: `enc:Claim` and its closed kinds
(Finding, Hypothesis, Suggestion…). Its own description says it "names the root the reflection: source lattice
deliberately lacks, at the enc: level where discourse needs it" — the resource a demonstrative («these findings») can
bind, whatever its epistemic source.

`enc:EncodedClaim : reflection:DerivedResource` sits on both: Derived by construction (a program parsed it), and
carrying its discourse kind as a second `is_a` (D68 §2, multi-class inhabitation). The two axes are orthogonal and
should stay so. A Finding can be Declared, Derived or Verified; the discourse kind says nothing about the warrant.

**D72 adds a third distinction that belongs in this picture.** `declared_by` (who asserted) and `warranted_by` (what
grounds it) were conflated in one string slot until this week. The warrant axis is the informal precursor of a
justification term: `warranted_by = wrn:warrant_selective_essentiality_criterion` says *this claim rests on that
criterion* in exactly the sense `JustifiedBy(j, P)` says it formally — but as an unchecked pointer rather than a typed
derivation.

That suggests the target shape, and it is the most useful thing this document has to say about direction:

> A claim's `warranted_by` is a **stub for a justification term**. The system's trajectory is to replace informal
> warrant pointers with `JustifiedBy` certificates as the corresponding reasoning is formalized — the warrant naming
> what the certificate will eventually prove.

Nothing needs to change today for that to be true; it means `warranted_by` should not be given semantics that compete
with `JustifiedBy`, and that a claim carrying both should be understood as one formalized and one not-yet.

## 8. Invariants this document asserts

1. `JustifiedBy(j, P)` and `JustifiedBy(j, Q)` are the same type only when `P` and `Q` are convertible. (#137)
2. Every proposition slot holds a `Prop`. (#175, #191)
3. The type is writable in the surface language. (§3, unfixed)
4. A constructor's conclusion is computed by the checker from its arguments. (already true)
5. `IsVerifiedAs(iri, P)` holds only when the attached proof's statement translates to `P`. (§5, unfixed)
6. The epistemic axis and the discourse axis are independent; neither constrains the other. (§7)

## 8a. Relationship to D39

[D39](d39-justification-logic.md) is the design of the term language and the institution; this document is about the
discipline around its edges. It does not reopen D39's settled choices:

- The J-family positioning (§10), the exclusion of `!` and of a `Refutation` constructor, and the decision that
  witnesses are kernel-internal and admitted as a consequence of a trace-emitting commit (§11) all stand.
- D39 already anticipates §7's point in its own terms: "the type-theoretic surface (`ChainWitness.IsDerivedAs` vs
  `IsVerifiedAs`) makes the distinction explicit; the validator does not need to encode the foreign institutions'
  logics."

Two places where this document changes something D39 says:

1. **§10's factivity parenthetical is unearned** until #159 closes (§6).
2. **D39 §11 says "no kernel changes driven by D39"** — the kernel "sees `JustifiedBy` as an ordinary inductive type".
   That is exactly why (a)–(c) failed: an ordinary indexed family whose indices conversion ignored, whose proposition
   slot went unchecked, and whose type cannot be written. The requirements in §2 are kernel obligations that D39's
   non-goal did not anticipate, because it assumed the kernel's treatment of ordinary inductives was already sound.

## 9. Open questions

1. **Should D39 §10's factivity sentence be amended now or when #159 closes?** §6. As written it asserts a property
   the system does not have. Amending it costs nothing; leaving it means the design doc overstates a guarantee for as
   long as #159 is open.
2. **Where does the Lean/EigenTT statement comparison happen**, and is the translation trusted or checked? §5.2. This
   is D30's territory and it gates #159.
3. **Should `spec_str` generalize beyond `core:string`?** It is monomorphic today; numeric and structural
   specialization were deferred to the measurement-statistics institution.
4. **Is internalization a property we intend to hold?** Artemov Thm 2.14: with an axiomatically appropriate CS,
   anything provable has a term. Eigenius has no statement of this, and it is the property that would let "the chain
   establishes P" imply "some witness exists for P". Worth deciding whether it is a goal or explicitly not.
