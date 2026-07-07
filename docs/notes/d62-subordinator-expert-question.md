# Expert consultation — encoding clausal subordinators & discourse connectives in a dependent-type categorial grammar

*Context for: a researcher in categorial grammar and/or type-theoretic (Montague / MTT)
semantics. The question is about the right **semantic representation** for subordinating
conjunctions and discourse connectives in a system that deliberately keeps the
compositional layer thin and pushes inference to a separate reasoning layer.*

## Our system in one page

We are building a deterministic prose-to-typed-trees pipeline. The grammar engine is a
**dependent categorial grammar** in the Chatzikyriakidis & Luo tradition (dependent type
semantics in a Modern Type Theory), running over our own intensional type theory
("EigenTT" — an intuitionistic dependent type theory with a `Prop` universe, inductive
types, and opaque axioms; `¬P := P → False`).

Salient design commitments:

- **Categories → types via a homomorphism `⟦·⟧ : Cat → Type`.** Syntactic categories
  (`NP`, `S[mood,fin]`, slashes) are reflected as a first-class `Cat` datatype; `⟦·⟧`
  maps each to an EigenTT type (e.g. `⟦S⟧ = Prop`, `⟦N⟧ = A → Prop`, functors to arrow
  types). Combinators are forward/backward application (+ limited composition and lexical
  type-raising).

- **The kernel is a *felicity oracle*, not a generator.** A lexical entry carries a
  category `cat`, a semantics `sem`, and its declared type `sem_type`. The entry is
  admitted only if it passes a **felicity gate**: `⟦cat⟧` is definitionally equal to
  `sem_type` *and* `sem` type-checks at that type. An untrusted proposer (an LLM, or a
  lexicon import) only ever *proposes* entries/parses; the kernel admits or rejects. A
  parse that does not type-check has no reading.

- **Two-level semantics — thin compositional core, rich reasoning layer.** The
  compositional `sem` we build during parsing is deliberately *minimal*. Logical and
  discourse meaning that is **flavor- or context-dependent** is **not** baked into the
  lexicon; it is supplied downstream by a separate **reasoning institution based on
  justification logic** (Artemov-style `t : F`, "term `t` justifies formula `F`"), which
  adds axioms/inference per flavor and per domain. Felicity (type-checking) is explicitly
  **not** truth: whether a proposition *holds* is a separate, witnessed grounding
  judgment.

We have already settled two precedents with this philosophy, and the subordinator
question is whether/how to extend it:

1. **Coordination (`and`/`or`) is a schematic parser rule, not lexical.** Because
   coordination is polymorphic over categories (`X CONJ X → X` for any `X`), and `⟦·⟧`
   has no denotation for a category *variable*, `and`/`or` cannot be lexical entries that
   pass the felicity gate. They are handled by a dedicated combinator: same-category,
   `Prop`-ending ("conjoinable", Partee & Rooth) constituents combine via pointwise-lifted
   generalized conjunction (`and → ∧`, `or → ∨`), with NP coordination producing a
   member-retaining group. This matches the standard CCG treatment of coordination as
   schematic.

2. **Modal auxiliaries are *opaque* unary operators.** `can/could/may/might → Possible`,
   `must → Necessary`, and (just added) `will → Will`, `would → Would`,
   `should → Should` — each an **opaque `Prop → Prop` axiom**, *not* a Kripke/possible-
   worlds encoding. The kernel asserts **no** modal laws (no T/4/5, no `◇ ↔ ¬□¬` duality):
   these are flavor-dependent (deontic `□` fails T; `will` is future, `would` conditional,
   `should` weak-deontic — none reduce to alethic ◇/□), so we keep each modal a distinct
   primitive and let the justification-logic reasoning layer supply the laws and relations.
   The guiding rule was *"don't pre-commit to a lossy collapse; preserve the distinction
   so the reasoning layer can recover the flavor faithfully."*

## What we want to encode next

Clause-level **subordinating conjunctions** — `because`, `although`, `while`, `if` — and
**contrastive `but`**, over a fixed domain of typed predications (e.g.
`affects(brca1, hela) : Prop`). We surveyed the OpenCCG English grammar (`core-en`): it
gives `but` a sentential-binary category, `although`/`if` an "initial" subordinator
category, `while` a "medial" one; each connective projects its **own stem-named binary
relation** `rel(Arg1, Arg2)` in the logical form (it never collapses `but` to `and`). It
has no entry for `because`, and treats `however`/`thus` as a separate *transitional
adverb* class.

Our **tentative** plan, by analogy to the modal decision:

- Lexicalize each binary subordinator as a concrete category `(S\S)/S`
  (`type Prop → Prop → Prop`), e.g. medial *"S₁ because S₂"* →
  `Because(⟦S₁⟧, ⟦S₂⟧) : Prop`, where `Because` is an **opaque** binary `Prop → Prop →
  Prop` axiom, with causal/concessive meaning left to the justification-logic layer.
- `but` likewise becomes a lexical `(S\S)/S` with its own opaque relation, rather than
  joining the schematic coordination rule.
- Defer `however`/`thus` (treat as discourse-anaphoric sentence adverbs later) and
  `to`+infinitive (infinitival complementation — separate construction).

## Our questions

We would value an expert opinion on whether this is the right shape, and specifically:

1. **Lexical concrete category vs. schematic rule for subordinators.** Is
   `(S\S)/S` (and a fronted `(S/S)/S` variant) the right categorial treatment, and is it
   correct that subordination — unlike coordination — should be *lexical* (concrete
   category) rather than a schematic combinator, because it is not category-polymorphic?
   Are there constructions where this under-generates?

2. **Opaque binary relation vs. logically-structured connective.** We propose one opaque
   `Op(p, q) : Prop` per connective, with all real inference deferred. Where does this
   break down?
   - **`if` worries us most.** In a dependent type theory the natural reading of
     *"if P then Q"* is the function type `P → Q` (or a modal/generic conditional), which
     is *first-class*, not opaque. Lumping `if` with `because`/`although` as an opaque
     binary feels wrong. Should `if` be peeled off and given the `→`/Π treatment (or a
     restrictor-of-a-modal treatment à la Kratzer), while the others stay opaque?
   - **`but` = `and` + contrast.** Truth-conditionally `but` is `∧`. Should it *reduce*
     to our existing `And` with a separate, non-truth-conditional adversative annotation
     (so downstream consumers that ignore discourse relations still get the conjunction),
     or be a fully opaque `But(p, q)`? What do you lose either way?
   - **`because`/`although` are not truth-functional and are factive/presuppositional**
     (both clauses are presupposed; only the main clause is at-issue). An opaque
     `Because(p, q)` records neither the factivity nor the at-issue/not-at-issue split.
     In a proof-relevant / dependent-type setting, how would you represent the asymmetry
     and the presupposition projection — and is it a mistake to defer *all* of that to a
     separate reasoning layer rather than encoding it compositionally?

3. **Symmetry/asymmetry and information structure.** Coordination is symmetric;
   subordination is asymmetric (main vs. subordinate, backgrounded clause). Initial vs.
   medial position (*"Because S₂, S₁"* vs *"S₁ because S₂"*) — is that purely
   information-structural with identical truth-conditional/discourse content, or is there
   a semantic difference we'd be wrong to flatten? (We currently tokenize away commas, so
   our first cut is medial, comma-free only.)

4. **Discourse connectives (`however`, `thus`, `therefore`).** These relate a clause to
   *prior discourse* rather than to a second given clause. Is the sentence-adverb `S/S`
   treatment adequate, or do they require a genuinely **anaphoric** account (a free
   discourse-referent variable resolved like a pronoun) — i.e. should they route through
   the same anaphora-resolution machinery we are building for pronouns, rather than the
   subordinator machinery?

5. **The two-level architecture itself.** Is a thin compositional layer (typed,
   opaque connectives) + a separate justification-logic reasoning layer that supplies the
   logical/discourse content a sound and precedented division of labor? How does it
   compare to accounts that put the discourse relations *in* the compositional semantics
   (SDRT / Asher & Lascarides; dynamic semantics — DRT, Kamp; Heim's file change /
   anaphora), and what specifically do we forfeit by deferring? Is there prior work
   pairing a Modern-Type-Theory compositional semantics with a justification-logic (or
   other proof-term) inference layer for connectives and modality that we should read?

6. **Anything we are clearly getting wrong**, or a construction in this fragment
   (counterfactual `would`…`if`, concessive scope, `because` taking a speech-act vs.
   propositional argument — "epistemic/speech-act because") whose semantics will not
   survive the opaque-binary treatment and should shape the design now rather than later.

*(We will never knowingly ship a lossy collapse to save effort — the whole point of the
pipeline is faithful encoding — so we would rather get the shape right than minimal.)*
