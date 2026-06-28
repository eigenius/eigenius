# Reply to the expert — round 3

This nails it. We're taking all three: the **plug** correction for attitude verbs, the
**two-overloaded-lexemes** treatment of `because`, and your `But` analysis. One adjustment
on `But` forced by our kernel, then your follow-up.

## `But` — we land on Option A, because Option B needs a kernel feature we don't have

EigenTT has opaque *axioms* (an IRI with no body, inert under normalization) but **no
controllable-unfolding / `irreducible` marker on definitions**. So Option B (`But := And`,
never-unfold-in-parser) would require building a general opacity mechanism first. We may do
that eventually — it's independently useful — but it's bigger than this slice.

So for now: a distinct inductive `But (P Q : Prop) : Prop` with `And`'s single pair
constructor (hence identical projections — our FraCaS-style conjunction elimination still
gives `X but Y ⊨ X` and `⊨ Y`), plus your coercion `forget_contrast : But(P,Q) → And(P,Q)`
as a globally available axiom for the identity/subsumption cases. The tag is preserved in
the AST; the reasoning layer applies the coercion explicitly when it needs to unify with
`And`. The one cost vs. B is that propositional anaphora to a `but`-conjunction must travel
through the coercion rather than seeing definitional identity — acceptable, and it fits our
"explicit, witnessed" stance. *Does Option A's explicit coercion lose anything you'd
consider load-bearing, or is it a clean substitute here?*

## Your follow-up: what happens when an exported obligation fails to ground?

This is the center of our whole design, so the answer is sharp: **a presupposition that is
demonstrably false fails closed.** Our system's governing rule is that an unsupported
result is never silently dropped, weakened, or routed around — it is *recorded as a
finding* and it blocks. Concretely, every exported obligation `h_p : p` is discharged by a
**graded, witnessed** grounding judgment:

- **Holds** — a witness grounds `p` (graded Observed / Derived / Verified by witness
  strength). The obligation is discharged; the utterance is felicitous.
- **Open** — no witness yet. A genuine discovery gap; the utterance is *not* asserted as
  grounded — it waits.
- **Fails** — the KB grounds `¬p` (a witness of the negation). This is **presupposition
  failure**: not a truth-value gap we paper over, but a recorded finding that the
  utterance's presupposition is contradicted. The commit gate rejects; the finding names
  the contradicting witness.

So *"the knockout mice died because the gene was deleted"* when the KB demonstrably holds
the gene was **not** deleted does not yield a quietly-false causal claim — it yields a
recorded presupposition-failure finding pointing at the contradicting fact. Felicity
(type-checking) admitted the *form*; grounding is where it dies, loudly.

The one principled exception is **local accommodation**, and it ties directly back to your
plugs correction: under negation or inside an attitude verb, the obligation can be
discharged **locally** (λ-bound within that operator's sub-derivation) rather than projected
to the global Γ — i.e. *"Smith believes it died because the gene was deleted"* confines the
(possibly false) presupposition to Smith's belief state, so the global KB's `¬p` does not
trigger a global failure. In our terms: **projection vs. local accommodation = whether the
plug discharges the hypothesis**, exactly the mechanism you described for attitude verbs.
Global free obligation + KB-`¬p` ⇒ fail-closed finding; locally-discharged obligation ⇒ the
failure is scoped to that operator and handled there.

**Our questions:**

1. Is binding the complement's obligations the *standard* way to get local accommodation,
   or is local accommodation usually a separate, optional repair distinct from the
   plug-binding of attitudes (i.e. should *negation* optionally bind, as a marked reading,
   while *attitudes* always bind)?
2. When an attitude verb binds the obligations into the subject's epistemic state, those
   discharged hypotheses presumably become obligations *relative to a belief context*
   rather than the KB. Does that just mean the justification-logic layer carries a
   per-agent context stack, with the same Holds/Open/Fails discipline applied against the
   agent's beliefs instead of the global KB?
