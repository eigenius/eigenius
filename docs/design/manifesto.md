# The Eigenius Manifesto

## Knowledge deserves better than probability.

We are building the most powerful reasoning tools in human history on a foundation that cannot tell truth from plausibility. Large language models produce text that reads like knowledge — fluent, confident, well-cited — but carries no guarantee that any of it is correct. There is no structural mechanism, anywhere in the architecture of these systems, that distinguishes a valid derivation from a convincing hallucination.

This was an acceptable limitation when LLMs were novelties. It is not acceptable when they are embedded in the workflows that design drugs, advance physics, build infrastructure, and engineer autonomous systems. We are now making high-stakes decisions informed by tools that are constitutionally incapable of telling us when they are wrong.

We refuse to accept this as a permanent condition.

## The epistemic crisis is architectural, not algorithmic.

Scaling models does not solve this problem. Neither does better training data, reinforcement learning from human feedback, or more sophisticated prompting. These approaches improve the *probability* that an output is correct. They do not provide *certainty*. They do not provide *proof*. They do not even provide a reliable way to measure how far from certain we are.

The problem is not that LLMs are bad at reasoning. The problem is that the infrastructure surrounding them has no concept of what reasoning *is*. Orchestration frameworks chain LLM calls together without any type system governing what flows between them. Knowledge graphs store claims without storing the derivations that produced them. RAG systems retrieve documents without verifying that the model uses them correctly. The entire stack treats knowledge as text and reasoning as string concatenation.

We believe this is a solvable engineering problem, and we are building the solution.

## Four kinds of knowledge.

Not all knowledge is created equal. A measured experimental result, a computational derivation, and a mathematically proved theorem are fundamentally different things. They deserve different levels of trust, and any system that collapses them into the same undifferentiated representation is lying about what it knows.

Eigenius maintains four epistemic categories as a first-class architectural concern:

**Declared knowledge** has authority but no evidence. It entered the system because a human decided it should be there — a load requirement of 5kN, a target safety factor of 2.0, an ontology definition, a regulatory threshold. The system vouches for its well-formedness: the declaration is structurally valid, the types are correct, the required fields are present. It does not vouch for its truth. Declared knowledge expresses intent, policy, or convention. It is the starting point from which everything else is measured, and it is the weakest epistemic claim the system makes — because it rests on human judgment alone.

**Observed knowledge** has provenance. It came from somewhere — a measurement, a paper, a dataset — and the system records where. The system does not claim it is true. It claims it was recorded, and it can tell you by whom, when, and from what source.

**Derived knowledge** has a derivation. It was produced by a typed processing pipeline from other knowledge, and the entire chain — every input, every step, every intermediate result — is recorded, replayable, and queryable. You can ask "what assumptions does this conclusion depend on?" and get a complete, typed answer. Not a summary. Not a guess. A structural accounting.

**Verified knowledge** has a proof. A machine-checked certificate that the conclusion follows from the premises by the rules of mathematics. Not probably follows. Not follows according to the training distribution. Follows in the way that 2+2=4 follows from the axioms of arithmetic.

These distinctions are not metadata. They are not tags that a user applies. They are computed from the provenance chain of every resource in the system, enforced by the type system, and queryable at any time. The system always tells you what it knows, how it knows it, and how confident you should be.

## To prove something is to construct the evidence for it.

There is a deep philosophical idea at the heart of Eigenius, and it predates AI by a century.

In mainstream mathematics, you can prove that something exists without showing what it is. You can demonstrate that a solution *must* exist by assuming it doesn't and deriving a contradiction. This is sufficient for pure mathematics, but it is useless for engineering. Knowing that a drug binding site *must* exist somewhere on a protein tells you nothing about where it is, how to reach it, or what molecule might fit.

Constructive mathematics takes a different position: to prove that something exists, you must construct it. To prove that a function has a particular property, you must exhibit the function and demonstrate the property computationally. Every proof is also a program. Every verified claim carries within it the computational evidence for its own correctness.

This is the mathematical tradition that Eigenius is built on. When the system verifies a derivation, it does not merely assert that the conclusion is true. It constructs and preserves the evidence — a computational witness that can be inspected, replayed, and mechanically checked. This is the deepest possible contrast with large language models, which assert fluently but construct nothing.

The practical consequence: when Eigenius marks a conclusion as *verified*, any person or machine can independently re-examine the proof and confirm its validity. The trust is not in the system. The trust is in the mathematics. And the mathematics is open for inspection.

## Science and engineering demand this.

We are not building Eigenius because the mathematics is elegant, although it is. We are building it because the problems that matter most are the ones where getting the answer wrong is catastrophic, and where the reasoning chains are too long and too complex for any human to hold in their head.

**Quantum physics** pushes the limits of mathematical reasoning. A derivation in quantum error correction may span hundreds of steps across linear algebra, probability theory, and information theory. When an LLM assists with such a derivation, the question is not "does this sound right?" The question is "is this right?" Only mechanically checked proof answers that question.

**Drug discovery** requires tracing inference chains across molecular biology, chemistry, pharmacology, and clinical medicine — each with different models, different uncertainty profiles, different standards of evidence. When a pharmaceutical team reviews a candidate, they need to know not just what the model predicts, but why it predicts it, what the prediction depends on, and which of those dependencies have been independently verified. This is not a luxury. It is the difference between a successful drug and a billion-dollar failure.

**Autonomous systems** — from humanoid robots to self-driving vehicles — integrate mechanical engineering, control theory, perception, planning, and real-time decision-making. The challenge is not just building each subsystem but maintaining coherence across all of them. Does the control model match the mechanical design? Do the safety constraints account for every failure mode? In a typed knowledge graph, inconsistencies across domains become structural errors that the system catches — not surprises that surface during integration testing, or worse, in the field.

**Complex infrastructure** — airports, railway stations, power grids — routinely exceeds human ability to track dependencies and verify specifications. Berlin Brandenburg Airport was delayed nine years. Stuttgart 21 has consumed decades of engineering effort against cascading specification changes. These are not failures of competence. They are failures of epistemic infrastructure: the knowledge existed, scattered across thousands of documents and models, but no system could trace dependencies, verify consistency, or answer "if this changes, what else breaks?"

## Our commitments.

**We will never pretend that probable means certain.** The system will always show you the epistemic status of every conclusion. If something is derived but not verified, it will say so. If something depends on an unverified assumption, it will say so. Epistemic honesty is not a feature we can toggle off for convenience.

**We will make provenance mandatory, not optional.** Every resource in the knowledge graph will have a traceable origin. There will be no mechanism for inserting knowledge without recording where it came from. This costs storage and complexity. We pay that cost willingly.

**We will make verification incremental, not all-or-nothing.** The system will be useful before any formal proofs exist. Recorded observations, typed pipelines, and reasoning traces are valuable on their own. Formal verification deepens over time, applied first to the conclusions that matter most. The architecture will always show you where the boundary between verified and unverified lies, and it will provide a continuous path to push that boundary forward.

**We will build on open foundations.** The platform is open source. The kernel's formal properties are publicly specified and verifiable. The type system is grounded in standard mathematics — the same constructive type theory that underlies the proof assistants used by mathematicians and software verification engineers worldwide. We are building a substrate, not a product. Research communities should be able to build on it, extend it, verify it, and trust it precisely because they can inspect every layer.

**We will not compromise the type system for adoption.** It would be easier to build a system that accepts untyped data and provides weaker guarantees. We will not do this. The type system is the mechanism that makes epistemic categories enforceable rather than aspirational. Weakening it would make the system more convenient and less honest. We choose honesty.

## What we are building.

Eigenius is a platform for AI-driven science and engineering. At its core: a typed knowledge graph where every resource carries tracked provenance and queryable epistemic status. A type system that validates processing pipelines before execution, guaranteeing that they are well-formed and terminating. A reflection layer that captures every reasoning step — including every LLM invocation — as a typed, auditable record. A capability protocol that allows formal proof checking to be applied incrementally to the conclusions that matter most.

For those who work in formal methods: the pipeline type system is founded on dependent type theory — a fragment of the Calculus of Inductive Constructions, the same foundation as Lean 4. The kernel is implemented in Rust with Verus proof annotations. The formal specification track uses Lean 4, and the path from type-checked to formally proved is continuous by design. The storage layer is built on TiKV for distributed deployments, with computation and storage cleanly separated. We have made real architectural choices, and they are documented in the open.

For everyone else: what matters is the consequence. When you ask the system "is this conclusion verified?", the answer is grounded in mathematics, not in a model's confidence score. And the mathematics is open for anyone to inspect.

## The future we are working toward.

We envision a world where a researcher can ask: "Show me everything we know about this protein's binding behavior — what is observed, what is derived, and what is proved — and show me the unverified assumptions that the derived conclusions depend on." And the system answers completely, correctly, and auditably.

We envision a world where an engineering team can change a specification and immediately see every downstream dependency that is affected, every analysis that needs revalidation, and every certification that is invalidated — as a structured query, not as a six-month manual review.

We envision a world where the boundary between what AI has verified and what AI has merely asserted is always visible, always queryable, and never ambiguous.

The ambition is not to replace human judgment. It is to give human judgment the infrastructure it needs to operate reliably at the scale of problems that now exceed unaided human comprehension.

We are building that world. Join us.

---

*Eigenius is open source under the Apache 2.0 license.*
*[eigenius.io](https://eigenius.io)*
