# D88 — Four questions the justification layer leaves open

*Status: **answered** `2026-09-05` · design note.*

*Follows [D87](d87-the-verification-judgement.md) §7 and [D49](d49-chainwitness-machinery.md)
(largely superseded). Governing document:
[Judgements, Warrants, and Logics](judgements-and-warrants.tex).*

---

## Summary

| | question | answer | basis |
|---|---|---|---|
| §1 | Do the `witness:Is*As` types earn their place, or should the check key on the constructor? | **They earn it — on layering, not soundness.** The type declares the trigger and carries the lookup's parameters as its indices, so the kernel needs no knowledge of the justification vocabulary | derived from the code |
| §2 | Should `justification:Term` merge into `justification:Certificate`? | **Yes, and merged `2026-09-05`.** The paper specifies term *shapes* and the typing relation `t : F`, not an encoding; the code had no use for the separate term index | measured, then built |
| §3 | Should a term name a chain instance by reference rather than by string? | **The string is sediment.** The leaf already behaves as a reference — `core:mentions` indexes it — but by a prefix heuristic rather than a declared type. Give it one | derived from the index |
| §4 | Can `app`'s `forall`-bound arguments be inferred? | **`app` yes, and built `2026-09-05`; `spec_poly` no.** | derived from `nbe/unify.rs`, then built |

---

## 1. The witness types earn their place

D87 §7 rules out deleting `Certificate.verified`'s premise: the constructor becomes unconditional
and `Verified` becomes assertable by anyone writing a certificate. It leaves open a third option —
keep the condition, drop the type, and check `verified(iri, P)` by a rule keyed on the
**constructor**. The witness looks like a side condition in all but name: the author elides it
(`declared(RULE, RULE_P)`, two arguments for a three-argument constructor),
`CheckHooks::synthesize_chain_witness` fills it, and nothing persists it.

### What the witness type does

Checking a certificate reaches `check_inductive_ctor_args`, which walks the constructor's arguments
and evaluates each declared argument type. At the elided slot that value is

```
Val::InductiveType { decl: witness:IsDeclaredAs, indices: [LitString(iri), P] }
```

and `synthesize_chain_witness` does three things with it (`check_hooks.rs:44-88`):

| | |
|---|---|
| **trigger** | `chain_witness_category_for_iri(decl.iri)` — anything else returns `Ok(None)` and the argument is checked normally. The type alone decides whether a chain query happens |
| **slot** | the argument exists because the premise is declared, and that is what makes the constructor conditional |
| **parameters** | `indices[0]` must be a `LitString` (the resource IRI), `indices[1]` is read back as the proposition. **`(iri, P)` reach the lookup as the type's own indices** |

The third job is the one that is easy to miss. The kernel never reads the constructor's arguments to
find out what to look up — it reads the *type's* indices, and validates their shape as it goes.

### This is not a soundness question

Both `core-ontology.json` and `justification.esl` are bootstrap files the kernel ships. Editing
either is a change to the kernel's own vocabulary by someone who could equally edit the kernel, so
neither mechanism sits on a different side of a trust boundary from the other. The paper's TCB —
*"the kernel's native type checker, each hosted external proof checker, each formal comorphism, and
the constant specification governing attributions"* — is about what can make a false proposition
accepted from outside, and a bootstrap edit is not in that model.

So the question is a layering and maintenance question, and answering it in soundness language would
be dressing it up.

### The argument: the type declares what the kernel would otherwise assume

`synthesize_chain_witness` learns everything it needs from the type: whether to fire, from
`decl.iri`; what to look up, from `indices[0]` and `indices[1]`; and whether those are well-shaped,
by checking them. All of that is **declared in core**, which the kernel owns.

Keyed on the constructor, the kernel would instead name `justification:Certificate.verified` — a
declaration in a layer above it — and read `(iri, P)` off argument positions, which nothing declares.
`witness_index.rs` crosses that line exactly once today, for `justification:Conclusion`, and marks it
as an exception: *"the D49 witness machinery is the one kernel site that is intrinsically
reasoning-aware."* A constructor-keyed rule would make the exception the mechanism.

**And that is what distinguishes §1 from §2 and §3.** Those collapse genuine duplication — the term
index and the untyped leaf each restate something another artifact already carries. The witness type
restates nothing: it is the only place the trigger and the lookup's parameters are written down.
Removing it does not delete a duplicate, it moves a declaration into kernel code as a hard-coded
name.

### What drift looks like, as a maintenance property

Not a soundness argument, and worth stating separately for that reason. "Drift" is the kernel's
expectation and the bootstrap ontology ceasing to agree.

**With the type**, every mode is a named error and two stop the process before a certificate is ever
checked:

| edit | what happens |
|---|---|
| `witness:IsDeclaredAs` renamed or removed in core | `Certificate.declared`'s premise names it, the reference does not resolve, the inductive declaration cannot be built, and **bootstrap fails** |
| a third index added | the premise still resolves; the hook answers *"ChainWitness predicate `…` expected 2 indices (iri, P), got 3"* — the guard `justification.esl`'s header calls *"the chain ontology drifted from the kernel's expectation"* |
| the indices reordered | *"iri index must be LitString, got …"* |

**Keyed on the constructor**, a rename makes the match fail. There is no premise left to fill, so
nothing is missing and nothing errors: `verified(iri, P)` type-checks and
`Certificate(Verified(iri), P)` is inhabited for any proposition. The refactor that caused it looks
like it worked.

**Scope of the claim.** Nothing checks that `Certificate.verified` *has* a premise, so the type does
not protect against `justification.esl` dropping it outright.

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

**And the one thing that looked like a blocker is not one.** The paper states that *"the Verified
state corresponds to the configuration lacking the middle layer: the system holds
`Judgement(L, t, P)` directly."* That describes the **state** — a claim whose warrant is a checked
proof carries a `prov:judgement` on its trace and no certificate at all, which is exactly what the
D87 §6 fixture does. It says nothing about the `verified` **constructor**, which is how a
*downstream* claim cites such a claim as a ground (D87 §9.2: *"`Certificate(Verified(c), P)` is what
a downstream claim cites … `c`'s own standing comes from its `justification:proof` judgement, not
from a certificate over itself"*). State and constructor are different things, and the merge changes
neither.

### What the merge cost, measured after the fact

The sentence that stood here — *"the demo notebook's certificate is the largest authored artifact
that would be rewritten"* — **was wrong by about sixty**. The notebook held 12 of 663 argument sites.

| | sites | compiled by the suite |
|---|---|---|
| WRN publication chain (6 files) | 610 | yes |
| benchmark tracer chains, kernel fixtures, `prose-to-formulas-v2` | 41 | yes |
| the notebook | 12 | no |

Plus 176 dead `justification:App` / `Sum` alias bindings, which compiled only because an unused
alias is never elaborated. Nothing could be deferred: `wrn_phase3` and `wrn_phase5` load the
publication chain.

The scale is also what made the change safe. An arity survey ran first: 306 `app` calls all at
arity 6, `spec_poly` uniformly at 5, every off-pattern count inside a `//` comment. That made the
transformation five positional rules rather than a judgement per site, applied by a paren-aware
transformer deleting argument spans right-to-left, leaving formatting and comments untouched. The
check was the existing tests — `wrn_phase3` asserts that `app(declared(plan), observed(input))`
reaches *verified* with both witnesses chain-resident — so a semantically wrong rewrite fails an
assertion instead of compiling quietly. Exactly one non-mechanical assertion broke: the
`core:mentions` test, which is the claim of this section.

What the estimate should have counted is every file the suite compiles, found with one
extension-agnostic grep. Scoping to `kernel/tests/fixtures/*.esl` and the notebook missed the
publication chain, a fixture under `crates/`, and — three separate times — ESL embedded in Rust
string literals, which no `.esl` glob reaches.

**Residue:** 32 alias bindings of the form `x = Declared(IRI)` in 7 files still name deleted
constructors. They are inert. Remove them by hand, one file at a time with a compile check, not by
a pattern sweep.

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

### What was built, `2026-09-05`

The §2 merge landed first and removed `j1` / `j2`, so the constructor reaching this work was
`app : forall (A : Prop, B : Prop) => Certificate(A -> B) -> Certificate(A) -> Certificate(B)`.
`B` is still the result index; `A` still occurs nowhere in it.

**Declared, not derived.** `core:implicit_args` lists the binder NAMES a constructor's author does
not write, and `implicit(A, B)` is the ESL clause that sets it. A binder is elided because the
declaration says so — never because the checker turns out to be able to solve it. An earlier
attempt derived implicitness from solvability and shifted the author's arguments onto the slots it
elided: `verified(CLAIM, P)` put a string where a `Prop` belonged, because which slot an argument
lands on then depends on what the solver managed.

Names rather than positions because a positional list cannot be checked — every index is in range
for *some* telescope — while a name that binds nothing, or binds twice, is an error at both the
compiler and the decoder. The clause sits on the constructor rather than marking `{A : Prop}`
inside the `forall`, because per-constructor is the only scope in which EigenTT represents
implicitness: `InductiveCtorDecl::implicit` is per-constructor and `Exp::Pi` has no binder style.
Brace syntax inside a general `forall` would have to be rejected everywhere else — surface syntax
promising something the type theory does not have.

**Two kernel changes were needed. The fifth piece named above was one of them; the other is not on that list.**

| | |
|---|---|
| a `MetaCtx` outliving one unification | Phase F's stated scope, as predicted. One context now spans the whole constructor check, so a binder can be solved from the result type up front *and* from an argument as the loop reaches it |
| unification comparing two anonymous arrows componentwise | **not predicted.** `Certificate(A -> B)` puts the metas inside a `Val::Pi`, and every `Val::Pi` fell through to readback equality, which cannot solve one |

**Restricted to anonymous arrows, and that restriction is the soundness argument.** A `Patt::Unit`
binder cannot be referenced, so neither codomain mentions it, so no variable is introduced and both
sides are compared at the same level — there is nothing a solution could capture. A named binder
still falls through to `eq_nf`. The restriction is also why this is not a behaviour change for
meta-free types: readback preserves `Patt::Unit` for D49's witness-key byte stability, so for two
anonymous arrows readback equality already *is* componentwise equality, and the one pair the two
would judge differently — an anonymous arrow against a named-but-unused binder — is what the guard
excludes.

The metavariable scope rule was strengthened alongside it. `solve_meta`'s check was *"approximated
for v1 by accepting any reference"*; metas now record the level they were created at, and a solution
proposed from inside a binder the meta does not scope over is refused. Refused rather than
inspected: a `Val` hides variables inside closure environments, so "does this mention a variable
above level N" is not decidable by a structural walk, and a walk treating closures as opaque would
answer no for exactly the unsound cases. Nothing reaches it today — the arrow rule never raises the
level — which is the point: it is the invariant that a later descent into a dependent binder has to
satisfy, and it fires instead of capturing.

**A first attempt did descend under the binder.** Comparing codomains one level in refuses to solve
any meta from outside, which is correct but too strong: an `app` nested as the argument whose
inference fixes an enclosing `app`'s binder is elaborated with **no expected type**, so `B` is not
fixed up front either. `Certificate(A -> B)` carries both binders, and one comparison against the
argument's inferred type determines both — but only if the codomain can be solved, which under a
binder it cannot.

Twelve tests reject it, across every authored certificate corpus in the tree:

| | |
|---|---|
| `wrn_phase3` | 8 |
| `wrn_phase2`, `wrn_phase5`, `sab16_tracer`, `sab18_tracer` | 1 each |
| `eigenius-statistics` `d39_composition` | 1 |

The shape they all exercise is left-nested application — `cert2 = app(cert1, …)` through an `alias`
binding, where the argument that has to be inferred is itself an `app`. The WRN publication chain
carried 67 such sites against the two tracers' 14, which is why deleting the tracers later the same
day cost no coverage here.

**`spec_poly` stayed fully explicit, `T` included.** `T` reaches the index only inside `P(x)`.
Solving it from the premise argument fails for the same reason it fails from the result: the domain
of `forall (y : T) => P(y)` would fix `T`, but the codomain is `P(y)` — a meta applied to a bound
variable — and the whole argument type has to unify for any of it to count.

**Cost.** 309 `app` calls lost two arguments each across 14 ESL files, plus 2 `sum_l`, 1 `sum_r`,
and the notebook's 4. Same transformer as §2, with a comment-aware splitter this time: an argument
list containing `// outer A, B` splits on the comma inside the comment, which made the first arity
survey report three different arities for one uniform call shape.

## 5. Work this decides

Three of the four questions are answered *yes, change it*. What remains for each is implementation,
not deliberation.

| | change | cost |
|---|---|---|
| §2 | collapse `justification:Term` into `justification:Certificate`, leaving `Justification : Prop -> Type 2` | bootstrap edit to a versioned ADT, one reseed. `certificate_indices`' five call sites (three only test `.is_some()`), `support` / `is_fully_verified` / `wellfounded` walking the certificate value instead of the term index, and the demo notebook's certificate — which gets **smaller**, since it stops spelling out the term at every node |
| §3 | declare the leaf IRI-valued, on `Declared` / `Observed` / `Verified` / `Checked` | bootstrap edit, same reseed. The open sub-choice is `core:iri` as a DataType versus a format slot on `InductiveArgType` |
| §4 | infer `app`'s `forall`-bound arguments | **bootstrap edit**, same reseed. Implicitness must be *declared* — deriving it from solvability misaligns the author's remaining arguments (measured `2026-09-05`), so it needs a binder style on `Exp::Pi` through the D47 codec, ESL syntax, and marking the constructors |

§1 decides the opposite — the `witness:Is*As` types stay — so there is no work under it.

All three are bootstrap and should ride one reseed. §4 was thought independent of the other two; it
is not, and doing it *after* §2 also shrinks it, since the merge removes `j1` and `j2` from `app`
outright.

## 6. Genuinely open

Neither of these was asked here, and neither is answered.

1. **Widening the unification fragment.** `spec_poly`'s `P` needs higher-order unification, which
   D48 §3.1 excludes by design. Whether to widen it is a decision about EigenTT, not about the
   justification layer.
2. **One indexed witness family instead of three.** §1 establishes why the types exist; it says
   nothing about whether `IsDeclaredAs` / `IsObservedAs` / `IsVerifiedAs` should be
   `ChainWitness(category, iri, P)`, which would make `trace_category`'s mapping a value rather than
   three constants.

## 7. Out of scope

`justification:Certificate`'s seven constructors, `Sum`'s departure from LP's axiom, the
well-foundedness condition, and the three-level stratification are settled in the paper and
unaffected by any answer here.
