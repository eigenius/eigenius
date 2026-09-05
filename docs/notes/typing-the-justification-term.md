# Typing the justification term

> **Superseded in part by [D88](../design/d88-four-questions-the-justification-layer-leaves-open.md)
> §3** (`2026-09-05`). D88 checks each candidate reason for keeping the separate term index and finds
> none load-bearing in the implementation, so the decision is referred to the paper's author as a
> formalism question. **§2a.2 below is wrong** and D88 corrects it: the withdrawal scenario does not
> distinguish the two shapes — after a withdrawal, `holds(kernel, c, Certificate(j,P))` and
> `holds(kernel, c, Justification(P))` both stop validating. §4's instance-reference finding is
> answered in D88 §4, and §5.4's "separable ergonomic half" is answered in D88 §5, where it turns
> out to be the largest of the four items rather than the smallest.

*Open design question, raised `2026-09-05` while closing out
[D87](../design/d87-the-verification-judgement.md) §3.5. Concerns
[D39](../design/d39-justification-logic.md) §3 and §5 —
`justification:Term` and `justification:Certificate`.*

---

## 0. The observation

Three things now work, each argued separately and each landed:

- **Provenance grounds `Declared` and `Observed`.** A `prov:DeclarationTrace` or
  `prov:ObservationTrace` is what admits those witnesses; no grade is stored anywhere.
- **A `prov:VerificationTrace` is the record of a proof term having been type-checked**, and since
  D87 it carries the checker's own result as `prov:judgement` plus the inputs that make the verdict
  recomputable.
- **Propositions are represented properly** — `eigentt:Term`, the D47 codec,
  `reflection:canonical_proposition`, with proposition identity settled by `hash_proposition_exp`.

And yet the design keeps feeling circular. This note names why: **three parallel structures encode
one thing, and two of them are determined by the third.**

## 1. The redundancy, precisely

```
data justification:Term { Declared(core:string), Observed(core:string), Verified(core:string),
                          App(Term, Term), Sum(Term, Term) }

data justification:Certificate : justification:Term -> Prop -> Type 2 {
    app : forall (A : Prop, B : Prop, j1 : Term, j2 : Term) =>
          Certificate(j1, A -> B) -> Certificate(j2, A) -> Certificate(App(j1, j2), B),
    ...
}
```

**Every one of `app`'s four `forall`-bound arguments is inferable from its two explicit ones.**
Unifying against the types of the two certificate arguments determines `j1`, `j2`, `A` and `B`.
They are implicit arguments in the Lean/Agda sense, spelled explicitly — and the authoring cost is
visible in `notebooks/examples/stats-and-reasoning.json`, where a single certificate spells out
every proposition and every term at every node, six arguments per `app`.

**And the term index is determined by the certificate value.** Given `c1 : Certificate(j1, A -> B)`,
`j1` is fixed. So the term is a projection of the derivation, not independent data.

## 2. What the shape would be instead

One indexed inductive over the proposition alone:

```
data justification:Justification : Prop -> Type 2 {
    declared : forall (iri, P) => witness:IsDeclaredAs(iri, P) -> Justification(P),
    observed : ...,  verified : ...,
    app      : forall (A, B) => Justification(A -> B) -> Justification(A) -> Justification(B),
    sum_l / sum_r, spec_poly
}
```

The term stops being a separate object: the *value* of a `Justification(P)` is the derivation, and
its constructor tree is what `support` walks. This is the user's point stated as a type —
**the typing of a justification term should include the grounding and the proposition it grounds,
and composition should compute the resulting proposition.** `app` above does exactly that: `B` is
produced, not asserted.

## 2a. What the governing paper already says

`judgements-and-warrants.tex` is the authority here, and it uses **`JustifiedBy(j, P)`** — both
indices — in its own stratification:

```
Judgement(kernel, c, JustifiedBy(j, P))   a checker verified the certificate c
JustifiedBy(j, P)                          j grounds P
P                                          the proposition
```

So §2's merge is a **departure from the governing formalism**, not a cleanup within it. Three things
the paper says bear directly on it:

1. **The warrant is computed from the TERM.** §"Dynamic Computation of Warrants": *"The system
   explicitly stores provenance relations, justification terms, committed judgements, and the
   declared premises cited by those terms … and evaluates the warrant directly from the
   justification term."* Under the merge the retained object is the derivation, whose constructor
   tree is the term, so this is probably preserved — but it is the paper's stated primitive and the
   merge changes what is stored.

2. **Withdrawal is a first-class scenario.** *"If an accountable party withdraws the declaration
   `f : I → O`, every dependent conclusion automatically reverts to the underlying observations
   without requiring resource edits."* After a withdrawal the stored certificate no longer
   type-checks, while the term is still readable and the warrant recomputes to a weaker grade. The
   split makes "the record of what was cited" and "the check that passed at commit" separately
   durable. Under the merge they are one object whose type index becomes a lie — recoverable by
   walking the tree and ignoring the index, but that is a property to establish rather than assume.

3. **`Verified` has no middle layer at all.** *"The Verified state corresponds to the configuration
   lacking the middle layer: the system holds `Judgement(L, t, P)` directly."* This is exactly what
   D87 built — `prov:judgement` is `holds(logic_lean4, Checked(t), P)` on the trace, with no
   `JustifiedBy` in between. So for the strongest ground there is *no justification term*, and the
   merge would have to say what happens at that end.

## 3. What is actually lost — checked, not assumed

| the current design's reason | does it survive the merge? |
|---|---|
| D73 §1.2: the term is *retained whole* rather than collapsed to a scalar, so every epistemic question stays a query | **yes.** The retained object becomes the `Justification(P)` value; `support` walks its constructors instead of the term, returning the same leaf sets |
| two derivations of the same `P` must be distinguishable | **yes** — different values of the same type |
| `spec_poly` narrows the proposition and leaves the term alone, so specialising costs nothing in the audit trail | **at the leaf level, yes.** The value gains a node; the leaf set is unchanged, which is what `support` reports and what the notebook's claim ("the bridge reads as `Declared(bridge_iri)` before and after") is about |
| storage | **improves.** The term index disappears from the certificate's type |

No blocking objection found *within the current implementation*. §2a is where the objections
actually live, and they come from the governing paper rather than from the code.

## 4. The related defect: instances cannot be named from inside a term

`justification:Term`'s leaves are `core:string`, which looks like surface syntax leaking onto the
chain — the provenance axis moved off strings precisely because *"the string form could name an
origin but not link to it, so no query could reach the instrument, release or run behind an
observation."*

**It is forced, not sloppy.** The leaf is not a property of a resource, it is a subterm of an
encoded value: `Declared(iri)` sits inside `Certificate(Declared(iri), P)` inside `holds(...)`,
which is one `justification:judgement` blob. A reference from inside a term is a `ConstRef`, and
`resolve_const_ref` dispatches on the target's class — `core:Class`, `eigentt:Axiom`,
`core:InductiveType`, five primitives. **A `justification:Claim` instance is none of those.**
Declaring the leaf `core:resource` would not help: the triple index covers properties, and this is
not one, so it would advertise a graph edge that still does not exist.

**The same gap appeared three times in one batch**, which is why it is worth a note rather than
three local fixes:

| where | the instance that could not be named | what was done |
|---|---|---|
| `justification:Term` leaves | a `justification:Claim` | `LitString` |
| D87 §3.2 — the checked proof | a `lean:LeanProofPayload` | a new former, `Checked(payload_iri)`, taking `core:string` |
| D87 §6 / §3.1 — the demo fixture | `demo:lean:patient_1`, a `Patient` instance | **changed its class** to `eigentt:Axiom` so a proposition could mention it |

The rule underneath: *the term language can only name type-level declarations; every reference to a
chain instance enters as a string literal.* The fixture case is the tell — the proposition had to
quantify over all Patients rather than name one, and the fix was to move the individual into a
class the term language can see.

**What it costs today**, measured: no graph edge, so "which conclusions rest on this claim?" is not
a triple-index query — `wellfounded` re-parses each leaf with `Iri::parse` and calls
`layer.resolve` on every traversal (`wellfounded.rs:191`, `:130`). Referential integrity *is*
enforced, but by the witness: `synthesize_chain_witness` does the parse and errors on a bad IRI.
And `wellfounded` **skips** an unparseable leaf rather than rejecting it, leaning on the certificate
check having run first — a coupling worth knowing about.

`Checked(payload_iri)` is the first former whose whole purpose is to name a chain instance. It is
still a string inside, but it is a distinct constructor with its own check rule, which is the shape
a general instance-reference former would take.

## 5. What to settle before acting

1. **Does the ground category belong in the type too?** `support` and `is_fully_verified`
   distinguish leaf categories by walking. A type carrying the category would make "is this fully
   verified" a type-level property — but D73 §1.2's whole argument is that a grade is *computed,
   not stored*, and putting it in the type is close to storing it. Name the tension before
   resolving it.
2. **Is `spec_poly`'s universe constraint unchanged?** It binds `T : Type 1`, which is what forces
   `Type 2` (eigenius#188). The merged inductive inherits that; confirm nothing else moves.
3. **What migrates?** Measured `2026-09-04`: zero certificates cite `Certificate.verified` and zero
   resources carry `justification:proof`. The demo notebook's certificate is the largest authored
   artifact and would have to be rewritten.
4. **The ergonomic half is separable.** Making `app`'s four `forall`-bound arguments implicit is a
   pure inference change with no semantic content, and it removes most of the authoring burden on
   its own. It is worth doing — or ruling out — independently of the merge, and it would show
   whether the merge's remaining benefit is large enough to pay for a versioned ADT change.

5. **The paper is the authority, and it uses `JustifiedBy(j, P)`.** Changing the formalism means
   amending `judgements-and-warrants.tex`, not just the ontology. That is a higher bar than an
   implementation change and should be met deliberately.

**Do not treat §2 as decided.** D87 §7 asserted a conclusion of exactly this shape —
*"`witness:IsVerifiedAs` is removable"* — which did not survive derivation (§3.5 withdrew it). The
same happened to §3 of this note within an hour of writing it: it reported "no blocking objection
found", and reading the paper produced three considerations it had missed. Finding no objection is
not the same as showing there is none.
