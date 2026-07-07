# D62 2d — subordinators & connectives: design findings (expert-informed)

**Status:** Declared design anchors from an external expert consultation in categorial /
type-theoretic semantics, plus our synthesis. Supersedes the tentative "uniform opaque
binary" plan in [d62-encoding-implementation-plan.md](d62-encoding-implementation-plan.md)
§2d. The question that prompted this is in
[d62-subordinator-expert-question.md](d62-subordinator-expert-question.md). Not yet
implemented — the two forks in §4 are the user's calls.

## 1. What the expert confirmed

- **Subordinators are lexical, not schematic.** Concrete categories `(S\S)/S` (medial)
  and `(S/S)/S` (initial) are correct; subordination is not category-polymorphic the way
  coordination is. (Under-generation risk only in RNR/ellipsis — not worth distorting the
  baseline for.) *Our instinct was right.*
- **The two-level architecture is sound and precedented.** Thin compositional layer
  emitting (possibly underspecified) logical form + a separate inference layer mirrors
  **MRS** (HPSG) and **Glue Semantics** (LFG). Defensible and modern, especially for a
  deterministic pipeline.

## 2. What the expert overturned (per-connective revised treatment)

| word | tentative plan | **revised treatment** |
|---|---|---|
| `if` | opaque `If(p,q)` | **native implication `p → q`** (the dependent type theory's `→`/Π). An opaque `if` would defeat modus ponens in the checker — forfeiting the whole point of dependent types. Kratzer-restrictor for modal conditionals is a later refinement. |
| `but` | opaque `But(p,q)` | **truth-conditional `And(p,q)` + an orthogonal contrast tag.** A fully-opaque `but` hides from truth-only consumers that both conjuncts are asserted. Compositional layer builds the conjunction; the adversative relation goes to the discourse/reasoning layer. |
| `because`, `although`, `while` | opaque `Op(p,q):Prop→Prop→Prop` | **factive dependent signature requiring proofs:** `Because : Π(p q:Prop) → p → q → Prop`. The felicity gate then rejects a parse unless the context supplies (assumed) proofs of `p` and `q` — i.e. **presupposition becomes a type-checking requirement**, exactly our philosophy. |
| `however`, `thus`, `therefore` | sentence adverb `S/S` | **anaphoric — route through the pronoun/D64 machinery.** They introduce a *free propositional variable (+ its proof)* resolved against prior discourse, like a propositional pronoun. `S/S` is fine for the tree; the argument is anaphoric (DRT-style). |

**Position (medial vs. initial)** is truth-conditionally identical but affects anaphora &
focus (which clause is matrix/at-issue vs. backgrounded). v1 may flatten the *semantics*,
but the **derivation tree must preserve matrix-vs-subordinate topology** for the discourse
layer. (We tokenize commas away, so medial comma-free is the v1 surface form.)

## 3. Blind spots the expert flagged (shape the design now)

- **Speech-act `because`** — *"Are you going to the store, because we need milk."*
  `because` relates a proposition to a *speech act*, not `Prop → Prop`. Forcing
  `Prop → Prop` crashes felicity. Anticipate a `SpeechAct` universe.
- **Concessive/causal scope under negation** — *"He didn't leave because he was angry"*
  has the `¬(Because…)` reading (he left, but not for that reason) vs. the
  `Because(¬…, …)` reading. Needs careful syntactic bracketing; an opaque binary won't
  disambiguate.
- **Counterfactuals** (`would … if`) — require evaluating the antecedent in another
  world/context. EigenTT has no world/context indexing today → counterfactuals stay
  severely underspecified until it does.
- **Donkey anaphora** (the cost of deferring discourse to a separate layer) — purely
  compositional dynamic accounts (Ranta, *Type-Theoretical Grammar*, 1994; Σ-types thread
  discourse referents along the spine) get *"if a farmer owns a donkey, he beats it"* for
  free; our split makes it harder because the compositional layer doesn't dynamically
  extend the context with the antecedent's existential. Out of scope for this fragment;
  note it before relying on cross-clausal indefinite binding.

## 4. Architectural implications & the two open forks

The expert's recommendations converge on a **two-tier representation** (this is the SDRT
shape: propositional content + rhetorical relations):

- **Tier 1 — truth-conditional `Prop`** (compositional): native `→` for `if`, `And` for
  `but`, factive dependent operators for `because`/`although`/`while`.
- **Tier 2 — a discourse-relation channel** (reasoning side, justification logic):
  `but`'s contrast, `however`/`thus`'s rhetorical relations, modal laws, and discharge of
  the factive proof-obligations.

**Fork A — DECIDED: adopt the factive dependent signature now (v2).**
`Because : Π(p q:Prop) → p → q → Prop`. The principled shape (the expert's
recommendation; presupposition-as-felicity). It is a real **engine extension**: the parser
currently builds *closed* sem terms with `⟦S⟧ = Prop`; v2 requires threading *hypothetical
proof variables* through the categorial derivation (a clause must contribute not just
`p : Prop` but an assumed `h_p : p` in context). Touches `⟦·⟧` / the felicity gate, not
just a lexical row. **Gated on the expert confirming the §5 scoping account** (round-trip
in progress) before we build the engine extension — fail-closed: we asked the question, we
wait for the answer rather than building on an unvalidated account.

**Fork B — RESOLVED (better than either option offered): the discourse flavor lives in the
TERM CONSTRUCTOR, not a separate Tier-2 channel and not the type.** The expert's move: one
truth-conditional type `P ∧ Q` inhabited two ways — `AndIntro(p,q)` (plain) and
`ButIntro(p,q)` (contrastive). The type stays conjunction (truth-only consumers extract
both conjuncts); the constructor name is the reasoning layer's trigger for the adversative
axioms (defeasible `P ⤳ ¬Q`, focus alignment, QUD polarity). Zero retroactive parsing;
needs only that the AST preserve the constructor name.

*Translation into our propositions-as-types setting (to confirm with the expert).* Our
compositional clause sem is the **proposition** (`And(p,q)` is the inductive *type*, a
`Prop`), not a *proof term* — so "which constructor proved it" is not available as the tag
at parse time. The faithful analog is a **distinct proposition-former** `logic:But (P Q :
Prop) : Prop` with `logic:And`'s *exact* shape — the same single pair constructor, hence
the same left/right projections (`But(p,q) ⊨ p`, `⊨ q`), so it is truth-conditionally
conjunction — but a **distinct head** the reasoning layer pattern-matches. I.e. the tag is
the *type-former head*, not the proof constructor, because in our pipeline the clause sem
is a proposition. Same division of labor, different locus; raised in the reply.

## 5. Our answer to the expert's follow-up

> *"If you model factive subordinators by requiring proofs as arguments, how do you scope
> those proof variables when they interact with negation or modals in the downstream
> justification-logic layer?"*

The key move: **the proof arguments are hypothetical, not constructed.** The parser never
proves a clause (felicity ≠ truth — whether a `Prop` holds is a separate, witnessed
grounding judgment in our system). So the lexical entry for a factive subordinator
introduces *fresh proof variables* `h_p : p`, `h_q : q` into the typing context Γ and
builds `Because(p, q, h_p, h_q) : Prop`. Those free hypotheses **are** the factive
presuppositions; they leave the parser as **proof obligations exported to the
justification-logic layer** (to be discharged by a witness, or to fail closed as a
grounding gap).

Given that, **presupposition projection falls out of ordinary variable scoping**:

- A hypothesis that remains **free in the global context Γ** = a **projected**
  presupposition. A hypothesis **discharged (λ-bound) within a local sub-derivation** = a
  **filtered/local** presupposition. This maps Karttunen's plugs/filters/holes onto
  hypothesis discharge — machinery a natural-deduction / justification-logic layer already
  has.
- **Negation** (`¬X := X → False`) and **modals** (opaque `Prop → Prop`) are formed
  *within* Γ and **do not discharge** `h_p`/`h_q`. So the factive obligations **project
  through** negation and modals by default — the linguistically correct result
  (*"it must be because he was angry"* / *"it wasn't because he was angry"* both still
  presuppose he was angry).
- **Conditionals** (now native `→`) and other filtering environments correspond to **local
  discharge**: a presupposition of the consequent that depends on the antecedent is
  discharged by `→`-introduction — i.e. filtered, matching Karttunen.
- The negation-scope ambiguity (*"didn't leave because…"*) is then a **bracketing**
  choice — `¬Because(p,q,h_p,h_q)` vs. `Because(¬p, q, h_{¬p}, h_q)` — i.e. *where* the
  `Because` node sits relative to `¬`, with the proof obligations following their clause.
  Tier 1 records both readings as distinct derivations; Tier 2 discharges the obligations.

So in our system the expert's elegant idea is not just compatible but **identifies
presupposition with a proof obligation and projection with hypothesis scoping** — the
justification-logic layer is exactly the place those obligations are discharged. The open
cost is Fork A's engine work (threading the hypotheses through the derivation).

## 7. Round-3 expert outcomes

- **Projection account confirmed — except plugs.** Hypothesis-scoping = projection captures
  *filters* (conditionals) and *holes* (negation, modals) correctly, and **nested**
  projection works (Γ just accumulates a flat list of obligations). The gap is **plugs —
  propositional-attitude verbs** (`believes`/`claims`/`doubts`). If we treat them as opaque
  `Prop → Prop` operators they act like holes, so an embedded factive presupposition wrongly
  projects to the *author*'s context instead of staying in the subject's belief state (local
  accommodation). **Fix:** an attitude verb's lexical entry must **bind/discharge** the
  hypothetical proof variables its complement emits, evaluating them in a local epistemic
  context. ⚠️ **This affects our existing clausal-complement verbs** (`shows`, D63 §8.11),
  which are opaque report axioms today — they are plugs and need this once v2 lands.
- **`because` — two overloaded lexemes, not polymorphism.** `Because_prop : Π(p q:Prop) →
  p → q → Prop` and (deferred) `Because_act : Π(s:SpeechAct)(p:Prop) → s → p → Prop`. The
  deterministic felicity gate resolves the overload by the main clause's type — keeping
  types monomorphic and telling the reasoning layer exactly which relation it has. (We
  already carry multiple entries per surface form, so this is our native pattern.)
- **`but` — Option A (coercion), because Option B needs a kernel feature we lack.** EigenTT
  has opaque *axioms* but **no controllable-unfolding / `irreducible` marker**, so Option B
  (`But := And`, never-unfold) would require building that mechanism. Option A is supported
  today: a distinct inductive `logic:But (P Q:Prop):Prop` with `And`'s single pair
  constructor (⇒ identical left/right projections, so `X but Y ⊨ X` and `⊨ Y` both fire via
  the recursor — the FraCaS elimination still works), plus a coercion
  `axiom logic:forget_contrast : ∀(P Q:Prop) ⇒ logic:But(P,Q) → logic:And(P,Q)` for the
  identity/subsumption friction the expert flagged (intensional TT ⇒ `But(P,Q) ≢ And(P,Q)`;
  propositional anaphora / applying an `A∧B→C` theorem to a `But` need the explicit
  coercion). *Superseded — see the FINAL decision below.*
- **`but` — FINAL: maps to `logic:And` (contrast dropped, verified adequate).** Neither
  Option A (coercion) nor Option B (kernel transparency) nor even a distinct opaque
  `logic:But`: checking against the WRN source, **every `but` is "X but (not) Y"** —
  truth-conditionally plain conjunction, the contrast rhetorical and carried by explicit
  negation, so the adversative relation is **not part of the typed claim**. So `but`'s lexical
  `sem` is `λs₂.λs₁. logic:And(s₁, s₂)` — "S₁ but S₂" denotes the *same* proposition as the
  `and`-coordination of its clauses. Zero kernel change; no new axiom (`logic:But` dropped).
  The distinct contrast-preserving operator + kernel transparency
  ([#95](https://github.com/eigenius/eigenius/issues/95)) remain the documented upgrade for
  if/when discourse/argumentation structure becomes load-bearing (e.g. D61 faithfulness) —
  not needed for D62's prose→typed-claims goal.
- **Controllable unfolding (Option B) — re-assessed as cheap, filed as
  [#95](https://github.com/eigenius/eigenius/issues/95).** In NbE opacity = "evaluate to a
  neutral" (the `EigonAxiom` path), so conversion/readback handle it for free; given our
  decode-time name resolution the transparency knob lives at the decode boundary (strict →
  opaque neutral; unrestricted → unfold body), leaving eval/conv/readback untouched. The
  heavy lazy-delta / glued-values rewrite is only for *in-checker opportunistic* unfolding,
  which our parser=strict / reasoning=unrestricted split avoids. Build when the reasoning
  institution wants kernel-level transparency (a real consumer for the unrestricted half),
  on the first general reusable definition, or when whole-term-NbE conversion cost bites.
- **New follow-up to answer:** how do we resolve a free obligation that *fails to ground*
  (a presupposition demonstrably false per the KB)? — answered in the round-3 reply: it is
  the **fail-closed** discipline (presupposition failure → recorded finding, never silently
  dropped/accommodated; grade `Fails`), with *local accommodation* as the controlled
  exception tied to the plugs mechanism above.

## 6. Revised 2d slice order (post-decision)

1. **`if` → native `→`** — *implemented.* Lexical `(S\S)/S`, sem `λs₂.λs₁. s₂ → s₁`
   (*"S₁ if S₂"* ⇒ `⟦S₂⟧ → ⟦S₁⟧`). Non-factive, independent of both forks; uses the
   kernel's native arrow (no new logic type), highest type-theoretic payoff. Validates that
   native implication passes the felicity gate end-to-end.
2. **`but` → `logic:And`** (FINAL): verified adequate against every WRN `but` (all "X but (not)
   Y", truth-conditionally conjunction; contrast rhetorical, carried by explicit negation, not
   part of the typed claim). Lexical `(S\S)/S`, sem `λs₂.λs₁. logic:And(s₁, s₂)` — same
   proposition as `and`-coordination. Zero kernel change, no new axiom. Contrast-preserving
   `logic:But` + kernel transparency ([#95](https://github.com/eigenius/eigenius/issues/95))
   are the documented upgrade for when discourse structure is load-bearing.
3. **`because`/`although`/`while`** — Fork A = v2 factive-dependent; **gated on the expert
   confirming the §5 scoping account** + the proof-variable engine extension.
4. **`however`/`thus`** — anaphoric; deferred to the pronoun/D64 anaphora slice (2e).
5. Speech-act `because`, counterfactuals, donkey anaphora — flagged, out of fragment.
