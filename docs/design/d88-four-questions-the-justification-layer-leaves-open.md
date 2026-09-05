# D88 — Four questions the justification layer leaves open

*Status: **answered** `2026-09-05` · design note.*

*Follows [D87](d87-the-verification-judgement.md) §7, [D49](d49-chainwitness-machinery.md) (largely
superseded), and `docs/notes/typing-the-justification-term.md`. Governing document:
[Judgements, Warrants, and Logics](judgements-and-warrants.tex).*

---

## 1. Method, and why it is stated first

The `numeric-core-and-verification-judgement` batch produced three conclusions that did not survive
derivation:

| claim | what happened |
|---|---|
| D87 §7: `witness:IsVerifiedAs` is *removable* once `Certificate.verified` consumes the judgement | withdrawn — the premise is what makes the constructor conditional |
| the P7 closeout: the witness machinery is *no longer a soundness boundary* | over-generalised — the paper says postulation is **correct** for attributions and names the constant specification as part of the TCB |
| `typing-the-justification-term.md` §3: *no blocking objection found* | the paper produced three within the hour |
| this document's own §5, first version: *EigenTT has no implicit-argument mechanism at all* | wrong — `nbe/unify.rs` is 739 lines of pattern unification, already wired into every constructor check. The grep looked for `implicit` and `BinderStyle`; the module is called `unify` |

Each was plausible, stated as settled, and wrong in a way no test would catch. So every answer
below names the evidence that produced it, and says which of three kinds it is:

- **derived** — follows from the code or the paper, with the site named;
- **measured** — a fact about the tree at a stated date;
- **a decision that is not the code's to make** — where the answer belongs to the formalism, and
  this document says so rather than inventing one.

## 2. Do the `witness:Is*As` types earn their place? — **yes, derived**

**The question.** D87 §7 rules out deleting `Certificate.verified`'s premise (the constructor
becomes unconditional and `Verified` becomes assertable by anyone writing a certificate). It does
not rule out the third option: *keep the condition, drop the type* — check `verified(iri, P)`
against the chain by a rule keyed on the **constructor** rather than by filling an argument. The
witness is already a side condition in all but name: elided in the surface (`declared(RULE,
RULE_P)`, two arguments for a three-argument constructor), synthesised by
`CheckHooks::synthesize_chain_witness`, never persisted, carrying no information the trace does not.

**The answer turns on how drift fails, not on where the special case lives.**

Today the hook keys on the **expected type** being a witness-category inductive, and matches
`decl.name` against the three `witness:Is*As` IRIs, which are declared in `core-ontology.json`. The
premise `witness:IsDeclaredAs(iri, P)` appears in `justification.esl`'s constructor, so it must
*resolve*. If the kernel's expectation and the ontology drift apart, the constructor's premise names
a type that does not resolve and **the ontology fails to compile**. Loud, at build time.

Keyed on the constructor instead, the kernel would hard-code
`justification:Certificate.verified`. Rename or restructure that constructor and the kernel simply
stops matching: the side condition silently never fires, and `verified` is unconditional in effect
without anything saying so. **Silent, at run time, on the soundness-critical path.**

That asymmetry is the whole of it. The type is not carrying information; it is carrying a *resolution
obligation*, and the obligation is what makes drift loud. `justification.esl`'s own header records
the residue of the same concern — the hook *"still carries an 'the chain ontology drifted from the
kernel's expectation' guard."*

**What this does not claim.** The type does not protect against `justification.esl` dropping the
premise outright; nothing checks that `Certificate.verified` *has* one. Both files are bootstrap, so
both edits cost a reseed and neither is a user action. The claim is narrower and holds: of the two
mechanisms, one fails loudly under drift and the other fails silently.

## 3. Should the justification term merge into the certificate? — **not the code's decision**

**The question.** `justification:Term` and `justification:Certificate` encode one thing twice.
`app`'s term arguments are determined by its certificate arguments (`Certificate(j1, A -> B)` fixes
`j1`), so the term index is a projection of the derivation. A single indexed inductive
`Justification : Prop -> Type 2` would carry both.

**Measured `2026-09-05`: the implementation has no load-bearing use of the separate term index.**
Each candidate reason was checked and none holds:

| candidate reason | verdict |
|---|---|
| D73 §1.2 — the term is retained so warrant stays a query | the merged value's constructor tree **is** the term; `support` walks it and returns the same leaf sets |
| two derivations of one `P` must be distinguishable | different values of the same type |
| the paper's withdrawal scenario — a withdrawn declaration must not require resource edits | **does not distinguish them.** After withdrawal, `holds(kernel, c, Certificate(j,P))` and `holds(kernel, c, Justification(P))` both stop validating. `typing-the-justification-term.md` §2a.2 suggested this favoured the split; it does not, and that entry is corrected |
| `spec_poly` leaves the term untouched, so specialising costs nothing in the audit trail | preserved — `spec_poly` is not a leaf, so `support` yields the same set either way. The merged value gains a node; the audit trail does not change |
| something compares terms for equality | nothing found. `wellfounded` and `refutes` use leaf IRIs |

**So the answer is not in the code, and this document will not invent one.**
`judgements-and-warrants.tex` uses `JustifiedBy(j, P)` — both indices — in its own stratification,
and one further fact makes the merge a substantive formalism question rather than a refactor:
*"The Verified state corresponds to the configuration lacking the middle layer: the system holds
`Judgement(L, t, P)` directly."* The justification layer is entered only for `Declared` / `Observed`
grounds and their compositions; at the strongest ground there is **no justification term at all**. A
merged `Justification(P)` has to say what it means at that end, and that is a claim about the logic,
not about the implementation.

**Recommendation:** treat this as an amendment to the paper, to be made or declined by its author.
The implementation cost is real but secondary — the demo notebook's certificate is the largest
authored artifact and would be rewritten, and `justification:Term` is a versioned ADT.

## 4. Naming a chain instance from inside a term — **derived: the string is right, the principle is missing**

**The question.** `Declared(core:string)` looks like surface syntax on the chain. The provenance axis
moved off strings precisely because *"the string form could name an origin but not link to it."*

**Why a reference is unavailable, derived.** The leaf is a subterm of an encoded value, not a
property, so the triple index cannot see it whatever its declared type — declaring it
`core:resource` would advertise an edge that does not exist. A reference from *inside* a term is a
`ConstRef`, and `resolve_const_ref` dispatches on the target's class: `core:Class`, `eigentt:Axiom`,
`core:InductiveType`, five primitives. A `justification:Claim` instance is none of those.

**And that is by design, not by omission.** `eigentt:Term` is documented as *"the type-level subset
of EigenTT's Exp"*. Resource instances are outside that subset deliberately. `Exp::EigonResource`
exists in the kernel as a runtime value form and **has no D47 encoding** — verified: the encoder's
arms cover `Sort`, `Var`, `ConstRef`, `App`, `Ann`, `Pi`, `Sig`, `Lam`, `One`, `Id`, `UnitVal`, the
four literals, `CtorApp`, `Pair`, `Fst`, `Snd`, `Record`, `Refine` and `Checked`, and not
`EigonResource`. So there is no chain-mirrored form for an instance, and a string literal is what
remains.

**What is actually missing is a stated principle for integrity.** Three formers now name instances
by string, each with a different guarantee:

| former | integrity comes from |
|---|---|
| `Declared` / `Observed` / `Verified` leaves | the witness lookup — `synthesize_chain_witness` parses the IRI and errors on a bad one |
| `Checked(payload_iri)` (D87 §4.2) | construction — only the institution can build one, since `check` refuses the form |
| the D87 §6 fixture's subject | **class change** — `patient_1` was redeclared an `eigentt:Axiom` so the term language could see it |

The third is the tell: the fixture's proposition had to quantify over all Patients until the
individual's *class* was changed. **The rule is: the term language can only name type-level
declarations; every reference to an instance enters as a string whose integrity is the consuming
rule's problem.** That rule is now true in three places and written down in none. Stating it is the
answer; a general instance-reference former is not warranted by three cases with three different
consumers.

**One coupling to record.** `wellfounded` *skips* a leaf whose string will not parse as an IRI
(`wellfounded.rs:191`), relying on the certificate check having refused it first. The graph walk is
only sound because the witness lookup ran. That is fine and should be a comment, not a discovery.

## 5. Making `app`'s arguments inferable — **derived: mostly already built; `app` yes, `spec_poly` no**

**Correction.** The first version of this section said *"EigenTT has no implicit-argument mechanism
at all — no binder styles, no elaboration, no unifier"*, and concluded this was the largest of the
four. That measurement was wrong. It came from grepping `kernel/src/nbe/` and `kernel/src/esl/` for
`implicit` and `BinderStyle` and reading two hits as absence — but the unifier is named `unify` and
lives in `kernel/src/nbe/unify.rs`, which neither term finds. Concluding "no mechanism" from the
absence of two guessed names is not a measurement.

### What already exists

| | |
|---|---|
| **A unifier with metavariables** | `nbe/unify.rs`, 739 lines — *"D48 Phase C — first-order pattern unification for EigenTT"*, with `MetaCtx`, occurs-checking and the Miller pattern condition |
| **Index unification against the expected type** | already runs on **every** constructor check — D48 Phase D, `check_inductive_ctor_args` |
| **Parameters solved from the expected type** | already: *"Parameters come from the expected type"* |
| **Argument elision keyed on declared type** | already: trailing `ChainWitness`-typed slots are omitted by the author and filled by the kernel. This **is** an implicit argument, restricted to one type family |

So four of the five pieces are in place, and the fifth is named in the code:

> Phase D uses a fresh per-call `MetaCtx` — EigenTT doesn't yet have implicit-arg syntax that would
> create metas surviving outside ctor checking. **Phase F** (motive inference) will thread a
> longer-lived `MetaCtx` through.

The work is therefore *implicit-argument syntax plus a `MetaCtx` outliving one constructor check* —
which is Phase F's stated scope — and generalising the elision rule from "trailing
`ChainWitness`-typed" to "solvable by unification".

### Which binders, exactly

**`app` — reachable with what exists.**

```
app : forall (A : Prop, B : Prop, j1 : Term, j2 : Term) =>
      Certificate(j1, A -> B) -> Certificate(j2, A) -> Certificate(App(j1, j2), B)
```

Checking against an expected `Certificate(J, P)`:

- `j1`, `j2` — unify `App(j1, j2)` against `J`. First-order and structural: `J` is written by the
  author in the judgement's type and is literally an `App` node.
- `B` — unify against `P`. Immediate.
- `A` — **does not appear in the result type.** It occurs only in the argument types, so it cannot
  come from the expected type. It comes from *inferring* `c2 : Certificate(j2, A)` and reading the
  second index.

That last point is the one real change to the checking loop. `check_inductive_ctor_args` walks
arguments left to right in **check** mode against `arg_typ_val`, which presumes every earlier binder
is already solved. Solving `A` needs one argument elaborated in **inference** mode, with the result
unified back. Ordinary bidirectional elaboration, and no new theory.

**`spec_poly` — not reachable, and the reason is structural.**

```
spec_poly : forall (T : Type 1, P : T -> Prop, j : Term, x : T) =>
            Certificate(j, forall (y : T) => P(y)) -> Certificate(j, P(x))
```

`j` is first-order. But solving `P` means satisfying `P(x) ≡ Q` for a *concrete* `Q` with both `P`
and `x` unknown — **higher-order**, and outside Phase C's fragment by explicit design: D48 §3.1
restricts to first-order patterns, and `unify.rs` records that higher-order patterns are left to
*"the institution where Lean's elaborator handles higher-order"*.

So `spec_poly`'s `P` and `x` stay explicit unless the fragment is widened, which is a much larger
decision than this question.

### What this changes about the answer

Not the largest of the four — the smallest that yields anything, and partial. `app` is also the node
that repeats most in a deep composition (`stats-and-reasoning.json` nests four of them), so the
ergonomic win is concentrated exactly where the burden is. And it remains the only one of the four
needing no amendment to the paper.

## 6. What remains open after this

1. **§3 — the merge.** Referred to the paper's author, with the implementation evidence supplied
   and the withdrawal argument withdrawn.
2. **§5 — implicit arguments.** Scoped against what exists: Phase F's longer-lived `MetaCtx` plus
   implicit-arg syntax, generalising the elision rule the `ChainWitness` hook already implements.
   `app` is in reach; `spec_poly` needs a wider unification fragment and is a separate decision.
3. **The D87 §7 residue.** §2 answers *why the types exist*; it does not answer whether the three
   families should be one indexed family (`ChainWitness(category, iri, P)`), which would make
   `trace_category`'s mapping a value rather than three constants. Not examined here.
4. **`support` over a merged derivation** would need re-measuring if §3 proceeds; the current cost
   is over a small tree of IRIs, and the derivation is larger.

## 7. What this document does not touch

`justification:Certificate`'s seven constructors, `Sum`'s departure from LP's axiom, the well-founded
condition, and the three-level stratification are all settled in the paper and unaffected by any
answer above.
