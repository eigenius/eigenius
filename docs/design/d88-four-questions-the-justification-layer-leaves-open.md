# D88 — Four questions the justification layer leaves open

*Status: **answered** `2026-09-05` · design note.*

*Follows [D87](d87-the-verification-judgement.md) §7 and [D49](d49-chainwitness-machinery.md)
(largely superseded). Governing document:
[Judgements, Warrants, and Logics](judgements-and-warrants.tex).*

---

## Summary

| | question | answer | basis |
|---|---|---|---|
| §1 | Do the `witness:Is*As` types earn their place, or should the check key on the constructor? | **They earn it.** The type carries a resolution obligation, so kernel/ontology drift breaks the build instead of silently disabling the check | derived from the code |
| §2 | Should `justification:Term` merge into `justification:Certificate`? | **A decision for the paper.** No load-bearing use of the separate term index exists in the implementation, but the paper's formalism is `JustifiedBy(j, P)` | measured `2026-09-05`, then referred out |
| §3 | Should a term name a chain instance by reference rather than by string? | **The string is correct.** `eigentt:Term` is the type-level subset by design. What is missing is a stated rule about integrity | derived from the codec |
| §4 | Can `app`'s `forall`-bound arguments be inferred? | **`app` yes, `spec_poly` no.** Four of the five pieces already exist | derived from `nbe/unify.rs` |

---

## 1. The witness types earn their place

D87 §7 rules out deleting `Certificate.verified`'s premise: the constructor becomes unconditional
and `Verified` becomes assertable by anyone writing a certificate. It leaves open a third option —
keep the condition, drop the type, and check `verified(iri, P)` by a rule keyed on the
**constructor**. The witness is a side condition in all but name: the author elides it
(`declared(RULE, RULE_P)`, two arguments for a three-argument constructor),
`CheckHooks::synthesize_chain_witness` fills it, nothing persists it, and it carries no information
the trace does not.

**The answer turns on how drift fails.**

The hook keys on the expected **type** being a witness-category inductive, matching `decl.name`
against the three `witness:Is*As` IRIs declared in `core-ontology.json`. The constructor's premise
`witness:IsDeclaredAs(iri, P)` names that type, so it must resolve. If the kernel's expectation and
the ontology diverge, the premise names something that does not resolve and the ontology fails to
compile — loudly, at build time.

A constructor-keyed rule hard-codes `justification:Certificate.verified` instead. Rename or
restructure that constructor and the kernel stops matching: the side condition never fires, and
`verified` is unconditional in effect with nothing to say so — silently, at run time, on the
soundness-critical path.

The type carries a resolution obligation, not information, and the obligation is what makes drift
loud. `justification.esl`'s header records the same concern: the hook *"still carries an 'the chain
ontology drifted from the kernel's expectation' guard."*

**Scope of the claim.** Nothing checks that `Certificate.verified` *has* a premise, so the type does
not protect against `justification.esl` dropping it. Both files are bootstrap, so either edit costs
a reseed and neither is a user action. The claim is only that one mechanism fails loudly under drift
and the other fails silently.

## 2. Merging the term into the certificate is a decision for the paper

`justification:Term` and `justification:Certificate` encode one thing twice. `app`'s term arguments
are fixed by its certificate arguments — `Certificate(j1, A -> B)` determines `j1` — so the term
index is a projection of the derivation. A single indexed inductive `Justification : Prop -> Type 2`
carries both.

**Measured `2026-09-05`: no load-bearing use of the separate term index exists.** Five candidate
reasons, none of which holds:

| candidate reason | verdict |
|---|---|
| D73 §1.2 — the term is retained so warrant stays a query | the merged value's constructor tree **is** the term; `support` walks it for the same leaf sets |
| two derivations of one `P` must be distinguishable | they are different values of the same type |
| the paper's withdrawal scenario | does not distinguish them: after a withdrawal, `holds(kernel, c, Certificate(j,P))` and `holds(kernel, c, Justification(P))` both stop validating |
| `spec_poly` leaves the term untouched, so specialising costs nothing in the audit trail | `spec_poly` is not a leaf, so `support` yields the same set either way |
| something compares terms for equality | nothing does; `wellfounded` and `refutes` use leaf IRIs |

**The formalism decides it, not the code.** `judgements-and-warrants.tex` uses `JustifiedBy(j, P)`
with both indices, and states that *"the Verified state corresponds to the configuration lacking the
middle layer: the system holds `Judgement(L, t, P)` directly."* The justification layer applies to
`Declared` and `Observed` grounds and their compositions; at the strongest ground there is no
justification term at all. A merged `Justification(P)` has to say what it means there, which is a
claim about the logic rather than about the implementation.

**Recommendation:** treat it as an amendment to the paper, to be made or declined by its author.
Implementation cost is secondary — the demo notebook's certificate is the largest authored artifact
and would be rewritten, and `justification:Term` is a versioned ADT.

## 3. A term names an instance by string, and that is correct

`Declared(core:string)` reads like surface syntax on the chain, and the provenance axis moved off
strings for a stated reason: *"the string form could name an origin but not link to it."*

**A reference is unavailable here.** The leaf is a subterm of an encoded value rather than a
property, so the triple index cannot see it whatever its declared type — `core:resource` would
advertise an edge that does not exist. A reference from inside a term is a `ConstRef`, and
`resolve_const_ref` dispatches on the target's class: `core:Class`, `eigentt:Axiom`,
`core:InductiveType`, five primitives. A `justification:Claim` instance is none of those.

**That boundary is deliberate.** `eigentt:Term` is *"the type-level subset of EigenTT's Exp"*.
`Exp::EigonResource` exists in the kernel as a runtime value form and has no D47 encoding — the
encoder covers `Sort`, `Var`, `ConstRef`, `App`, `Ann`, `Pi`, `Sig`, `Lam`, `One`, `Id`, `UnitVal`,
the four literals, `CtorApp`, `Pair`, `Fst`, `Snd`, `Record`, `Refine` and `Checked`, and not
`EigonResource`. No chain-mirrored form for an instance exists, so a string literal is what remains.

**The gap is a stated rule, not a missing former.** Three formers name instances by string, each
with a different integrity guarantee:

| former | integrity comes from |
|---|---|
| `Declared` / `Observed` / `Verified` leaves | the witness lookup — `synthesize_chain_witness` parses the IRI and errors on a bad one |
| `Checked(payload_iri)` (D87 §4.2) | construction — only the institution builds one, because `check` refuses the form |
| the D87 §6 fixture's subject | a class change — `patient_1` is declared an `eigentt:Axiom` so the term language can see it |

The third case is the clearest: the fixture's proposition quantified over all Patients until the
individual's class changed. **The rule: the term language names only type-level declarations, and
every reference to an instance enters as a string whose integrity is the consuming rule's problem.**
Three cases with three different consumers do not warrant a general instance-reference former;
writing the rule down is the work.

**One coupling.** `wellfounded` skips a leaf whose string does not parse as an IRI
(`wellfounded.rs:191`), relying on the certificate check to have refused it. The graph walk is sound
only because the witness lookup ran, which belongs in a comment there.

## 4. `app`'s arguments can be inferred; `spec_poly`'s cannot

Four of the five pieces exist:

| | |
|---|---|
| a unifier with metavariables | `nbe/unify.rs` — *"D48 Phase C — first-order pattern unification for EigenTT"*, with `MetaCtx`, occurs check and the Miller pattern condition |
| index unification against the expected type | runs on every constructor check — D48 Phase D, `check_inductive_ctor_args` |
| parameters solved from the expected type | *"Parameters come from the expected type"* |
| argument elision keyed on declared type | trailing `ChainWitness`-typed slots: the author omits them, the kernel fills them. An implicit argument restricted to one type family |

The fifth is named in the code:

> Phase D uses a fresh per-call `MetaCtx` — EigenTT doesn't yet have implicit-arg syntax that would
> create metas surviving outside ctor checking. **Phase F** (motive inference) will thread a
> longer-lived `MetaCtx` through.

So the work is implicit-argument syntax plus a `MetaCtx` outliving one constructor check — Phase F's
stated scope — and generalising the elision rule from "trailing `ChainWitness`-typed" to "solvable
by unification".

**`app` is reachable.**

```
app : forall (A : Prop, B : Prop, j1 : Term, j2 : Term) =>
      Certificate(j1, A -> B) -> Certificate(j2, A) -> Certificate(App(j1, j2), B)
```

Against an expected `Certificate(J, P)`:

| binder | solved by |
|---|---|
| `j1`, `j2` | unifying `App(j1, j2)` against `J` — first-order and structural, since `J` is an `App` node the author wrote |
| `B` | unifying against `P` |
| `A` | **not in the result type.** It occurs only in argument types, so it comes from inferring `c2 : Certificate(j2, A)` and reading the second index |

`A` is the one change to the checking loop. `check_inductive_ctor_args` walks arguments left to
right in check mode against `arg_typ_val`, which presumes every earlier binder is solved; solving
`A` needs one argument elaborated in inference mode with the result unified back. That is ordinary
bidirectional elaboration and needs no new theory.

**`spec_poly` is not.**

```
spec_poly : forall (T : Type 1, P : T -> Prop, j : Term, x : T) =>
            Certificate(j, forall (y : T) => P(y)) -> Certificate(j, P(x))
```

`j` is first-order. Solving `P` means satisfying `P(x) ≡ Q` for concrete `Q` with `P` and `x` both
unknown, which is higher-order and outside Phase C's fragment by design: D48 §3.1 restricts to
first-order patterns, and `unify.rs` leaves higher-order patterns to *"the institution where Lean's
elaborator handles higher-order"*. `P` and `x` stay explicit unless that fragment widens, which is a
larger decision than this question.

**Scale.** This is the smallest of the four that yields anything, and the yield lands where the
burden is: `app` is the node that repeats, nested four deep in `stats-and-reasoning.json`. It is
also the only one of the four needing no amendment to the paper.

## 5. Still open

1. **The merge (§2)** — referred to the paper's author, with the implementation evidence above.
2. **Implicit arguments (§4)** — scoped, not designed. Phase F's `MetaCtx` plus implicit-arg syntax,
   generalising the elision rule the `ChainWitness` hook implements. `app` is in reach; `spec_poly`
   needs a wider unification fragment and is a separate decision.
3. **One indexed witness family.** §1 answers why the types exist, not whether the three should be
   `ChainWitness(category, iri, P)`, which would make `trace_category`'s mapping a value rather than
   three constants.
4. **`support` cost** — worth re-measuring if §2 proceeds. It currently walks a small tree of IRIs;
   a derivation is larger.

## 6. Out of scope

`justification:Certificate`'s seven constructors, `Sum`'s departure from LP's axiom, the
well-foundedness condition, and the three-level stratification are settled in the paper and
unaffected by any answer here.
