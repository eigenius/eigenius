# The epistemic stack

How a proposition, the grounds for it, a proof of it, and the logics that supply either fit
together — and where each one lives in the tree.

The design is [*Judgements, Warrants, and Logics*](../design/judgements-and-warrants.tex); its
conformance record is the implementation companion in `../publications/`. **This document is
neither.** It assumes the design and shows the machine: four strata, what each is for, and a worked
example of each taken from a chain that runs in CI today.

The per-piece guides go deeper on their own piece — [justification
logic](platform/justification-logic/), the [Lean institution](platform/lean-institution/), the
[statistics institution](platform/statistics-institution/), [composing
institutions](composition/). This is the one that says how they compose.

---

## 1. Four strata, and the rule that separates them

| stratum | form | reads | factive |
|---|---|---|---|
| proposition | `eigentt:proposition` on a resource | *this resource asserts P* | — |
| grounds | `justification:Grounds(P)` | *these are the grounds for a claim to P* | **no** |
| proof | `eigentt:Judgement` = `holds(L, t, P)` | *a checker of logic L verified t against P* | **yes** |
| warrant | computed, never stored | *what does this rest on?* | — |

**The one rule.** Nothing turns `Grounds(P)` into `P`. The types differ and no constructor, rule or
coercion relates them. That is what makes substituting grounds for a proof *inexpressible* rather
than discouraged, and it is the reason the two live in different families rather than in one slot
with a flag.

The composition is one-directional, each layer a statement about the one below:

```
Judgement(kernel, c, Grounds(P))   a checker verified that the grounds c ground a claim to P
Grounds(P)                         the grounds
P                                  the proposition
```

`Verified` is the configuration with **no middle layer**: the chain holds `Judgement(L, t, P)`
directly. That is why a proof judgement is not a stronger kind of grounds — it skips the stratum.

---

## 2. The proposition — `eigentt:proposition`

A proposition is a `Prop`-sorted term of the kernel's type theory, stored on a resource:

```esl
resource wrn:bridge_msi_selective : justification:Declaration {
    prov:was_attributed_to  = wrn:authors;
    prov:had_primary_source = wrn:warrant_selective_essentiality_criterion;
    prov:rationale   = "If a sample set measuring WRN dependency by MSI status has its MSI-group
                        mean strictly below its MSS-group mean (MSI more dependent), WRN is
                        selectively essential in MSI. The directional statistical claim is the
                        warrant.";

    eigentt:proposition = type_expr(
        stats:lt(stats:mean_diff_of("urn:eigenius:pub:wrn:wrn_dep_sampleset"), 0.0)
        ->
        onco:SelectivelyEssential("WRN", "MSI")
    );
}
```
<sub>`experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl:135`. The
`prov:rationale` is where the human-readable justification for the bridge lives; the proposition is
the machine-checkable half, and the two are deliberately separate.</sub>

Three things follow from it being a *term* rather than a string.

**It decides what warrant even applies to.** Warrant asks what evidence exists for a proposition, so
a resource carrying none cannot be asked — that is a category error, not an unanswered question. The
test is mechanical: does the resource carry `eigentt:proposition`. The property carries **no domain**
deliberately; constraining it by class would tie the warrant axis to a provenance fact.

**It is checked, not just held.** A property declaring `class_types eigentt:Term` is checked in CHECK
mode at commit — decode, check the type is a type, check the term against it. One uniform rule, no
per-slot exemption list.

**It lives in `eigentt:` and is declared in core.** The namespace says what it is; the layer is where
the cycle forces it. `core:param_kind` and `core:type_name` carry `Term` values, so `Term` itself had
to be in core — and since a namespace is a naming convention rather than a layer assertion, the
`eigentt:` name survives the placement. The EigenTT fragment was its own bootstrap layer until that
cycle made the boundary meaningless and it merged into core.

---

## 3. The grounds — `justification:Grounds`

`Grounds : Prop -> Type 2`. An inhabitant **is** the justification term: a constructor tree whose
leaves are the three categorical groundings and whose interior nodes compose them.

| constructor | what it does |
|---|---|
| `declared(iri, P)` | an accountable agent asserted P |
| `observed(iri, P)` | a recording occurred |
| `verified(iri, P)` | a checker verified a term against P |
| `app` | Artemov application: grounds for `A -> B` applied to grounds for `A` |
| `sum_l` / `sum_r` | two independent groundings; the constructor records which was preferred |
| `instantiate` | narrows a universal to an instance, adding no ground |

**Three grounds, not five.** `Computed` and `Sampled` are *readings of a term's shape*, not
categories: `Computed` is `app(declared(f : I -> O), observed(input))`, `Sampled` a bare `observed`
leaf. Neither is stored as a state name, and neither yields `Verified` — in `Computed` the typing
`f : I -> O` is itself declared.

**The leaves are not author-written.** Each grounding constructor consumes a `witness:Is*As`, a
zero-constructor inductive in `Prop` that ESL cannot inhabit. The kernel synthesizes it from the
layer's witness index at type-check time, keyed on `(category, iri, hash(P))`. So a certificate does
not *assert* that a claim is declared; it fails to check unless the chain already carries the trace
that admits the witness.

**`instantiate` is what lets one bridge serve a campaign.** In Artemov's own systems specialisation
is not a primitive — instantiation is a logical axiom, justified by a constant, applied with `·`. A
chain cannot take the constant specification to be *total* the way FOLP does, because every constant
here is a committed resource; `instantiate` is the chain-resident stand-in for that totality.

```esl
justification:grounds_judgement =
    holds( eigentt:logic_kernel,
           type_expr(app(
               instantiate(
                   core:string,                                    // T : the domain
                   fun (x : core:string) =>                        // P : core:string -> Prop
                       screen:HasLowIC50(x) -> screen:StrongInhibitor(x),
                   "urn:eigenius:demo:screen:EIG_0291",            // the instance
                   declared(                                       // grounds for the universal
                       "urn:eigenius:demo:screen:rule_strong",
                       forall (c : core:string) =>
                           screen:HasLowIC50(c) -> screen:StrongInhibitor(c),
                       Prop)),
               /* … the computed half: app(declared(plan), observed(sampleset)) … */)),
           type_expr(justification:Grounds(
               screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291"))) );
```
<sub>`kernel/tests/fixtures/universal_rule.esl:122`, which asserts this reaches `Holds`. Note the
binder names: the lambda binds `x`, the rule's own proposition binds `c`. That is deliberate — the
witness index hashes propositions **alpha-canonically**, so two alpha-equivalent propositions must
reach the same key or the `declared` leaf would find no witness.</sub>

The **warrant** is read off this term and never stored: `support`, `is_fully_verified`, `leaves_of`,
`survives_without`. A stored grade can contradict its own evidence; a computed one cannot, and it
stays current — withdraw a declaration and every dependent conclusion falls back to its observations
with no resource edited.

---

## 4. The proof — `eigentt:Judgement`

```
data Judgement { holds(logic : Logic, term : Term, type : Term) }
```

A judgement is the **checkable unit**, and the reification of justification logic's proof-checker
operator: LP's positive introspection `t:F -> !t:(t:F)` names the evidence a checker returns, and an
algebra with application and sum but no `!` runs the checker at commit and throws the result away.

The `logic` field is what makes the stack plural. For the kernel's own theory the checker is the
internal type checker and no translation is needed. For Lean it is a hosted Lean kernel, and a
comorphism relates the Lean proposition `P'` to the chain proposition `P`.

Two slots on a conclusion hold judgements, and conflating them is the substitution the whole design
forbids:

- **`grounds_judgement`** — `holds(kernel, c, Grounds(P))`. A checker verified the grounds. It does
  **not** say `P`.
- **`proof_judgement`** — `holds(L, t, P)`. A checker verified `t` against `P` itself. This is
  factive, and **only this admits a `Verified` witness**.

---

## 5. The logics — institutions

A participating logic supplies vocabulary, a decision procedure returning a tri-state verdict,
derivation resources, and *optionally* a judgement in its own logic. It does **not** assign a
warrant, define a witness type, or establish `Verified` on its own authority.

The operative question is not whether a logic satisfies the definition of an institution. It is
whether the system can **hold and re-check a witness** for what that logic establishes.

| institution | supplies a proof term? | ceiling |
|---|---|---|
| statistics | no — a satisfaction relation, no proof language | `Computed` |
| Lean 4 | yes — an export the host re-checks | `Verified` |
| the kernel | yes — it *is* the checker | `Verified` |

The statistics institution recomputes a declared plan against committed observations and returns
`Holds`. That verdict grounds nothing by itself: a computed claim rests on the plan being
**declared** to denote a function of its input — which no execution establishes — and on the input
being **observed**. So the ground is `app(declared(plan), observed(inputs))`, and neither half comes
from the run. The run record is provenance.

An institution may veto a commit on its own authority, but may not establish `Verified` on it.

---

## 6. Lean 4 — the one that closes the loop

The Lean institution is where a proof term actually enters, and it is worth following end to end
because every guard in the design shows up in it.

1. A `lean:LeanProofTerm` lands. AutoOnLoad fires `qc_proof_check`.
2. **Proof validity** — `nanoda_lib`, a Lean kernel reimplementation linked into the host,
   re-checks every declaration in the verbatim export, refusing any axiom outside the permitted set.
3. **Statement correspondence** — the claim's `eigentt:proposition` is externalized to a Lean `Expr`
   and compared to the target declaration's type with `def_eq`. Without this, `Holds` would mean only
   *"a theorem with this name type-checks"*.
4. The institution emits a `prov:VerificationTrace` carrying **`prov:judgement`** —
   `holds(logic_lean4, Checked(payload), P)` — plus the permitted axiom set, the checker identity and
   the checked declaration.
5. `witness_admission` reads that judgement's own **type** and admits a `Verified` witness keyed on
   `hash(P)`.
6. A downstream certificate writes `verified(claim_iri, P)`; the kernel synthesizes the witness from
   the index, and the certificate type-checks.

<sub>`crates/eigenius-lean/tests/notebook_fixture_test.rs` runs exactly this, including a **near
miss**: the same proof and target declaration bound to the claim about the *other* individual, which
must be refused. Without a fixture that fails, the demo shows the plumbing running rather than the
check discriminating.</sub>

**Why the trace carries the judgement rather than a note.** Before it did, the checker's result was
computed and discarded, and `Verified` was admitted on the strength of a record that a check *ran*.
Keying off the judgement's type makes the grade rest on what was checked.

**Why an author cannot write one.** `Exp::Checked(iri)` names a proof an external checker verified.
The kernel refuses it: it has no proof of the proposition and will not manufacture one. An
institution-emitted judgement is never asked, because `structural_validate` runs before
`autoonload_dispatch` and the followup pipeline slice has no validation phase. So "kernel-only,
refused from input" falls out of the pipeline's shape rather than from a guard.

**Where the trust actually sits.** Not in the refusal — in the record. The export bytes, the target
name, the proposition, the permitted axioms and the checker identity are all on the chain, so any
party can re-run the check and get the same verdict. That is stronger than a receipt: a receipt says
*I checked*; this says *check it yourself*.

---

## 7. The whole thing, on a real claim

The WRN chain encodes a published result. Its flagship discovery conclusion — WRN is selectively
essential in MSI cancers — projects like this, and these are assertions from
`kernel/tests/justification_projection.rs`, not an illustration:

| question | API | answer |
|---|---|---|
| how many independent ways to get here? | `support(t).len()` | **1** — no `Sum`, one chain |
| what does it rest on that nobody proved? | `leaves_of(t, Declared)` | `discovery_rule`, `dd_achilles`, `dd_drive` |
| what measurements of its own? | `leaves_of(t, Observed)` | **none** |
| is it fully verified? | `is_fully_verified(t)` | **false** |
| would it survive losing any one ground? | `survives_without(t, …)` | **no**, for all three |

Read that last column as the design working rather than as a disappointment. **The flagship claim of
a published paper rests on three declarations, zero observations of its own, and no proof at all —
and the chain says so out loud.** Two of those three are recompute results the authors declare denote
a function of their inputs; the third is the literature rule. Every one is load-bearing, because with
no `Sum` in the term there is no alternative route.

A stored grade could not tell you any of this. It would say "Derived" and stop. The counterfactual is
the argument for keeping the term rather than a scalar: authored with a fallback —
`Sum(dd_achilles, dd_drive)` instead of needing both — the same conclusion answers
`survives_without` differently while grading identically. Only the term tells the two shapes apart.

And a ground it never cited costs nothing: `survives_without(t, "…:not_cited")` is true.

<sub>The `sum_l` strengthening is what makes those two alternatives mean something: committing a
`Sum` now requires certificates for **both** branches, so an alternative `support` reports is one
that was actually grounded rather than one an author asserted (`kernel/tests/sum_requires_both_branches.rs`).</sub>

---

## Where to look next

| you want | read |
|---|---|
| to author a conclusion | [justification-logic tutorial](platform/justification-logic/) §"Authoring your own conclusion" |
| the statistics half in detail | [composition §7](composition/07-stats-and-reasoning-walkthrough.md) |
| the Lean half in detail | [lean-institution guide](platform/lean-institution/) |
| the type theory underneath | ESL guide §4–§7, and D46 / D47 / D48 |
| what is specified but unbuilt | the implementation companion in `../publications/` |
