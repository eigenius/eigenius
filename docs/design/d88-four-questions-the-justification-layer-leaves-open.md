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
| §2 | Should `justification:Term` merge into `justification:Certificate`? | **Open, and nothing forbids it.** The paper specifies term *shapes* and the typing relation `t : F`, not an encoding; the code has no independent use for the separate term index. Both inputs point at the merge | measured `2026-09-05` |
| §3 | Should a term name a chain instance by reference rather than by string? | **The string is sediment.** The leaf already behaves as a reference — `core:mentions` indexes it — but by a prefix heuristic rather than a declared type. Give it one | derived from the index |
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

## 2. Nothing requires the separate term index — not the code, not the paper

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

**The paper does not specify the encoding.** It specifies two things, and neither dictates how many
indices the family carries:

- **A typing relation.** *"Justification logic replaces the modality … with the explicit typing
  `t : F`, read as the term `t` is a justification for `F`. The justification is an object in the
  language rather than external metadata."* And where the paper states the application rule
  operatively, it writes the premise as **`j₁ : (A → B)`** — a term with a type, not
  `JustifiedBy(j₁, A → B)`.
- **Term shapes.** `App(Declared(f : I → O), Observed(input))` for *Computed*, a bare `Observed`
  leaf for *Sampled*, `Sum` for alternatives. These are shapes of the justification term, and a
  merged value's constructor tree has exactly them.

`JustifiedBy(j, P)` is how the paper *names* that relation in its stratification table. Reading it as
a requirement that the inductive family be indexed by both `j` and `P` mistakes notation for an
encoding — the paper's own primitive is `t : F`, which is a one-index family whose inhabitants are
the terms. The phrase that does constrain the shape is *"an inductive family … defined over an
algebra of justification terms that supports application and sum"*: the term algebra must remain
identifiable as such, which a merged value's constructor tree satisfies.

**So both inputs point the same way.** The code has no use for the separate index, and the paper
does not require it. The merge is a live option rather than a departure, and the current two-index
encoding is a choice nothing on record argues for.

**What still has to be settled** is not the indexing but one question about the layer's extent. The
paper states that *"the Verified state corresponds to the configuration lacking the middle layer:
the system holds `Judgement(L, t, P)` directly."* The justification layer applies to `Declared` and
`Observed` grounds and their compositions; at the strongest ground there is no justification term at
all. A merged `Justification(P)` has to say what it means there — whether `Verified` stays outside
the family, or the family absorbs a case the paper puts outside it. That question is unchanged by
the encoding and would need answering either way.

Implementation cost does not weigh much: `justification:Term` is a versioned ADT, and the demo
notebook's certificate is the largest authored artifact that would be rewritten.

## 3. The string leaf is sediment; declare the type

`Declared(core:string)` names a resource with an untyped string. The IRI-ness is real, relied on in
three places, and declared nowhere.

**The leaf already behaves as a reference.** `core:mentions` indexes it: `json_mentions_of_value`
treats *"any string that parses as a `urn:` IRI"* as a mention, so a committed
`Declared("urn:eigenius:demo:screen:bridge_lowic50")` produces a `(R, core:mentions, bridge)` edge
and *which conclusions rest on this claim* is already a `scan_predicate_object` query
(`well_known.rs:435`).

**But it is recovered, not declared.** Three consumers each rediscover the same fact by a different
route:

| consumer | how it learns the string is an IRI |
|---|---|
| the `core:mentions` index | a **prefix heuristic** — `s.starts_with("urn:")`, which over-approximates: every urn-shaped string in any term becomes a mention whether or not it is a reference |
| `synthesize_chain_witness` | `Iri::parse` at certificate-check time, erroring on a malformed leaf |
| `wellfounded` | `Iri::parse` again on every traversal (`wellfounded.rs:191`), skipping what will not parse |

Nothing rejects `Declared("not an iri")` at commit on its own terms. It is caught only because a
certificate check happens to parse the leaf.

**The type system has the concept and cannot reach it here.** IRI-valued *properties* declare
themselves — `justification:subject_iri` and `justification:refutes` carry `format = formats:iri`.
A constructor argument cannot: `InductiveArgType` has `arg_name` and `type_name` and no format slot,
and no `core:iri` DataType exists. So a term's leaf has no way to say what every consumer assumes.

**`Checked(payload_iri)` (D87 §4.2) is the same sediment, added `2026-09-05`.** It names a
`lean:LeanProofPayload` and takes `core:string` for exactly the reason the older leaves do — because
nothing better was available. Adding a fourth instance of a pattern is the point at which the
pattern gets fixed rather than documented.

**The fix.** Give a constructor argument a way to be declared IRI-valued — either a `core:iri`
DataType or a format slot on `InductiveArgType` — and give it to `Declared`, `Observed`, `Verified`
and `Checked`. Then the index rule becomes exact instead of a prefix match, the validator rejects a
malformed leaf at commit instead of leaving it to whichever consumer parses first, and the three
consumers stop each inferring the same fact.

This is a versioned change to `justification:Term` plus a bootstrap edit, so it rides a reseed. That
is the cost of the fix, not an argument against it.

**Not a symptom:** `wellfounded` reads terms rather than the `core:mentions` index for a semantic
reason, not a missing edge — `mentions` *"records both branches' edges undifferentiated"*, so a cycle
walk over it would misread `Sum` (`wellfounded.rs:21`). That stays true whatever the leaf's type.

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

1. **The merge (§2)** — nothing forbids it; the remaining question is what the justification layer
   means at `Verified`, which the paper places outside it entirely. That question is independent of
   the encoding and would need answering either way.
2. **Implicit arguments (§4)** — scoped, not designed. Phase F's `MetaCtx` plus implicit-arg syntax,
   generalising the elision rule the `ChainWitness` hook implements. `app` is in reach; `spec_poly`
   needs a wider unification fragment and is a separate decision.
3. **The typed leaf (§3)** — scoped, not designed. Whether it is a `core:iri` DataType or a format
   slot on `InductiveArgType` is open; both are bootstrap edits.
4. **One indexed witness family.** §1 answers why the types exist, not whether the three should be
   `ChainWitness(category, iri, P)`, which would make `trace_category`'s mapping a value rather than
   three constants.
5. **`support` cost** — worth re-measuring if §2 proceeds. It currently walks a small tree of IRIs;
   a derivation is larger.

## 6. Out of scope

`justification:Certificate`'s seven constructors, `Sum`'s departure from LP's axiom, the
well-foundedness condition, and the three-level stratification are settled in the paper and
unaffected by any answer here.
