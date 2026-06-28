# Reply to the expert — round 2

Thank you — this was extremely useful and we've already acted on it: `if` is implemented as
native implication and type-checks through our felicity gate end-to-end. Two things below:
our answer to your follow-up question, and one clarification on the `but` proposal that
arises from a specific feature of our setting.

## Your follow-up: scoping the proof variables of factive subordinators under negation/modals

The key move in our architecture is that **the proof arguments are hypothetical, never
constructed by the parser.** We never prove a clause during parsing — in our system
felicity (type-checking) is explicitly *not* truth; whether a `Prop` actually holds is a
separate, witnessed grounding judgment. So the lexical entry for a factive subordinator
introduces *fresh* proof variables `h_p : p`, `h_q : q` into the typing context Γ and builds

```
Because(p, q, h_p, h_q) : Prop
```

Those free hypotheses **are** the factive presuppositions. They leave the parser as **proof
obligations exported to the justification-logic layer**, to be discharged by a witness — or
to fail closed as a grounding gap.

Given that, **presupposition projection falls out of ordinary variable scoping**:

- A hypothesis left **free in the global context Γ** is a **projected** presupposition; a
  hypothesis **discharged (λ-bound) within a local sub-derivation** is **filtered/local**.
  This maps Karttunen's plugs/filters/holes onto hypothesis discharge — machinery the
  natural-deduction / justification-logic layer already has.
- **Negation** (`¬X := X → False`) and **modals** (opaque `Prop → Prop`) are formed
  *within* Γ and do **not** discharge `h_p`/`h_q`, so the obligations **project through**
  them by default — the linguistically correct result ("it wasn't because he was angry" /
  "it must be because he was angry" both still presuppose he was angry).
- **Conditionals** — now native `→` — and other filtering environments are exactly **local
  discharge**: a presupposition of the consequent that depends on the antecedent is
  discharged by `→`-introduction (filtered), matching Karttunen.
- The negation-scope ambiguity ("didn't leave because…") is then a **bracketing** choice —
  `¬Because(p,q,h_p,h_q)` vs. `Because(¬p, q, h_{¬p}, h_q)` — i.e. where the `Because`
  node sits relative to `¬`, the proof obligations following their clause. We record both as
  distinct derivations; the reasoning layer discharges the obligations.

So your idea lands as more than compatible: **it identifies presupposition with a proof
obligation, and projection with hypothesis scoping**, and the justification-logic layer is
exactly where those obligations are discharged.

**Our questions on this:**

1. Does this hypothesis-scoping = projection account match how you'd expect plugs/filters
   to behave, and are there projection facts it gets *wrong* (e.g. presupposition of the
   subordinate clause itself containing a presupposition trigger — nested projection)?
2. For the **speech-act `because`** you flagged: if we add a `SpeechAct` universe, the
   factive obligation on the main clause presumably becomes an obligation about a
   *performed act* rather than a proposition. Is there a clean way to keep one `because`
   lexical entry polymorphic over `Prop` vs. `SpeechAct` arguments, or are these
   irreducibly two lexemes?

## A clarification on `but` — proposition-as-type vs. proof-term tag

We want to adopt your "encode the discourse relation in the term, not the type" proposal,
but it interacts with a feature of our system we should flag.

In our setting **the compositional semantics of a clause is the proposition itself — a
*type*, not a proof term.** Conjunction is the inductive type `And (P Q : Prop) : Prop`
(à la Lean's `And`), so "S₁ and S₂" denotes the *type* `And(⟦S₁⟧, ⟦S₂⟧) : Prop`. We do not
build a proof of it at parse time (again: felicity ≠ truth). So "which constructor proved
it" — your `AndIntro` vs. `ButIntro` — is not available to us as the tag, because no proof
constructor is applied during parsing.

Our faithful translation is therefore to push the tag **up one level**, onto the
*type-former head*: a **distinct proposition-former**

```
But (P Q : Prop) : Prop          -- same single pair constructor as And,
                                  -- hence the same left/right projections
```

`But(p,q)` has exactly `And`'s shape, so it is truth-conditionally conjunction (both
conjuncts recoverable by the parametric recursor / projections), but its **head** is a
distinct constructor the reasoning layer pattern-matches to fire the adversative axioms
(defeasible `p ⤳ ¬q`, focus alignment, QUD polarity). The division of labor is identical to
what you proposed; only the *locus* of the tag moves from the proof constructor to the
type-former, because in our pipeline the clause semantics is a proposition rather than a
proof.

**Our question:** does moving the discourse tag from the proof-term constructor to the
type-former head preserve everything you intended — in particular, do you see any case
where a *proof-level* tag (which two derivations of the *same* `And(p,q)` could carry
differently) does work that a *type-level* tag (`And` vs. `But` as distinct propositions)
cannot? Our concern is whether anything downstream needs to treat `but`-conjunction and
plain conjunction as *the same proposition* (e.g. for anaphora to a conjunctive antecedent,
or for a consistency check that should see them as identical) — in which case distinct
type-formers would wrongly separate them, and we'd want a definitional coercion
`But(p,q) ≡ And(p,q)` that the reasoning layer can still see through.
