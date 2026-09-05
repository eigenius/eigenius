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

## 5. Making `app`'s arguments inferable — **measured: not an ergonomic change**

**The question.** All four of `app`'s `forall`-bound arguments are determined by its two explicit
ones, and the demo notebook spells every one out — six arguments per node, propositions and terms
both, at every level of a deep composition.

**Measured `2026-09-05`: EigenTT has no implicit-argument mechanism at all.** No binder styles, no
elaboration, no unification-driven argument synthesis anywhere in `kernel/src/nbe/` or
`kernel/src/esl/`. (`default_binder_style` in the externalizer is nanoda's, on the Lean side.)

So this is not "make four binders implicit". It is **introducing implicit arguments and a unifier to
the kernel** — a core type-theory feature with its own design surface: which binders are implicit,
how they are solved, what happens when solving fails, how the diagnostics read, and what it does to
the D47 codec's round-trip. Previously filed as the small, separable half of §3; it is the largest
of the four.

**It is also the one with the clearest independent value.** The authoring burden it removes is
visible without any of the other three changing, and unlike §3 it needs no amendment to the paper.
If §3 is ever taken up, this is a prerequisite either way: a merged `Justification(P)` still binds
`A` and `B` in `app`, and they are still inferable.

## 6. What remains open after this

1. **§3 — the merge.** Referred to the paper's author, with the implementation evidence supplied
   and the withdrawal argument withdrawn.
2. **§5 — implicit arguments.** Scoped, not designed. Needs its own note.
3. **The D87 §7 residue.** §2 answers *why the types exist*; it does not answer whether the three
   families should be one indexed family (`ChainWitness(category, iri, P)`), which would make
   `trace_category`'s mapping a value rather than three constants. Not examined here.
4. **`support` over a merged derivation** would need re-measuring if §3 proceeds; the current cost
   is over a small tree of IRIs, and the derivation is larger.

## 7. What this document does not touch

`justification:Certificate`'s seven constructors, `Sum`'s departure from LP's axiom, the well-founded
condition, and the three-level stratification are all settled in the paper and unaffected by any
answer above.
