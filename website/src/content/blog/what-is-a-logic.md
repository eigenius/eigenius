---
title: "Part I — What Is a Logic?"
subtitle: "Institutions, Part I — The Anatomy Every Logic Shares"
description: "A logic is four parts and one law. Signatures, sentences, models, satisfaction — and the condition that ties them together. A self-contained conceptual foundation, with no category theory required."
pubDate: 2026-07-16
author: "Eigenius"
tags: ["institutions", "logic", "foundations"]
draft: false
series:
  name: "Institutions"
  part: 1
---

*A three-part introduction to the theory of institutions — the mathematics of what all logics have in common — and the case that it is the missing foundation for machine-checkable, AI-assisted science. This first part answers a deceptively simple question: what* is *a logic, really? Part II turns to translating between logics; Part III to making scientific reasoning checkable, and to the [Eigenius](https://eigenius.io/) project that is trying to build it.*

---

## 1. The problem: too many logics

If you build or analyze systems of any kind, you have almost certainly used more than one logic, even if nobody called them that. "Logic" here means something broader than the predicate calculus of a discrete-math course: it means any disciplined way of writing statements about systems and determining whether a given system satisfies them. Boolean algebra for digital circuits and first-order logic for database constraints qualify — but so do differential equations for continuous dynamics, process models and process calculi for interacting concurrent components, the state-variable reasoning of thermodynamics, quantum logic for quantum phenomena, and the inferential discipline of statistics (which, as we will see, is not the same thing as probability theory). Each has its own notion of what a statement is, what a system is, and what it means for a statement to hold of a system.

This diversity is not a defect — different phenomena genuinely call for different logics. Two things *are* defects. First, everything built on top of a logic tends to get rebuilt from scratch for each one: a theory of modular specifications for equational logic gets re-proved for first-order logic, then again for temporal logic; a theorem prover for one logic sits idle while people struggle in another where no good prover exists. Second, and increasingly pressing, modern engineered systems refuse to stay inside one logic at a time. A cyber-physical system has a controller specified in a discrete, temporal logic driving a plant governed by differential equations. A quantum computer has quantum dynamics below and classical control logic above. A data pipeline mixes statistical models with database constraints. Reasoning about the whole means coordinating several logics, and ad hoc coordination is where the errors live.

Institution theory, introduced by Joseph Goguen and Rod Burstall in the late 1970s, asks a simple question: **what do all logics have in common, and how much can we do using only that common core?** The answer turns out to be "a surprising amount": module systems, specification languages, and techniques for *borrowing* a theorem prover from one logic to solve problems in another can all be developed once, for every logic at the same time.

The word "institution" is admittedly odd. Think of it as *the institution of logic* — the formal framework within which a particular logical system operates, the way "the institution of marriage" is the framework within which particular marriages operate.

This part disassembles a logic into the four parts every logic shares, tests the definition against a running example, then shows the pattern turning up in corners of science you would not usually call "logic" at all — and closes with how sentences bundle into theories and modules. That is enough to stand on its own. The sequels build on it: Part II relates logics to one another, and Part III makes the whole edifice machine-checkable.

## 2. Disassembling a logic

An institution takes a logic apart into four components. Before the formal version, here is the plain-language inventory:

**Signatures — the vocabulary.** A signature is the collection of non-logical symbols currently in play: the function names, predicate names, sorts (types), constants. In a database logic, the signature is the schema. In a circuit logic, it is the set of wire and gate names. Crucially, the vocabulary can *change*: you can rename symbols, add new ones, or map one vocabulary into another. Institutions treat these vocabulary changes (called *signature morphisms*) as first-class citizens, because renaming and extension are exactly what happens when you compose modules.

**Sentences — the statements.** For each signature, there is a set of sentences you can write using that vocabulary: equations, first-order formulas, temporal assertions, whatever the logic offers. If you translate the vocabulary, every sentence translates along with it, in the same direction. Rename `plus` to `add` in the signature, and the equation `plus(x, 0) = x` becomes `add(x, 0) = x`.

**Models — the things statements are about.** For each signature, there is a collection of possible interpretations of that vocabulary: algebras, relational structures, state machines, traces — the "worlds" the sentences describe. Here something interesting happens: when you translate vocabulary from a smaller signature to a bigger one, models travel in the *opposite* direction. A model that interprets the big vocabulary automatically gives you a model of the small one — just ignore the extra symbols. This backwards flow of models is called the *reduct*, and the directional mismatch between sentences (forwards) and models (backwards) is the single most important structural fact in the whole subject. Almost every subtlety later on traces back to it — and in Part II it becomes the crux of how logics translate.

**Satisfaction — the truth relation.** Finally, for each signature there is a relation telling you which models satisfy which sentences: `M ⊨ φ`. This is Tarski's classical notion of truth, kept fully abstract — the institution does not care *how* satisfaction is defined, only that it exists.

And now the one and only axiom, which the founders called the **satisfaction condition**:

> **Truth is invariant under change of notation.**

Formally: if you translate a sentence along a vocabulary change and check it in a model of the bigger vocabulary, you get the same answer as if you had reduced the model to the smaller vocabulary and checked the original sentence there. Whether you move the sentence forward or the model backward, satisfaction agrees.

That's the entire definition. A logic, viewed as an institution, is: a collection of signatures with morphisms between them, sentences that translate forwards, models that translate backwards, and a satisfaction relation that is stable under translation. No assumptions about connectives, quantifiers, proof rules, or the internal structure of anything.

## 3. A running example

Abstract definitions stick better with one concrete instance carried all the way through. We will use **equational logic** — the logic of algebra — and later, in Part II, its big sibling, **first-order logic with equality**.

*Signatures.* An equational signature is a set of function symbols with arities. For example: a constant `0`, a unary symbol `s` (successor), and a binary symbol `+`. A signature morphism to a richer signature might rename these or map them into a larger vocabulary that also contains `*` and `max`.

*Sentences.* Sentences are universally quantified equations between terms: `∀n. n + 0 = n`, or `∀n,m. s(n) + m = s(n + m)`.

*Models.* A model (an *algebra*) is a set together with an actual function interpreting each symbol: the natural numbers with the real zero, successor, and addition — or, equally well, strings with the empty string, "append a character," and concatenation. The signature doesn't care which; that is the point of a vocabulary.

*Satisfaction.* An algebra satisfies an equation if the two sides evaluate to the same element for every assignment of values to the variables. The naturals satisfy `∀n. n + 0 = n`; they do not satisfy `∀n,m. n + m = n`.

*The satisfaction condition, verified informally.* Suppose the bigger signature has `+` and `*`, and we have a model of it — the naturals with both operations. Take the small-vocabulary equation `∀n. n + 0 = n`. Path one: translate the equation into the big vocabulary (it looks the same, since `+` maps to `+`) and check it against the full model. Path two: strip the model down by forgetting `*`, and check the original equation against what remains. Obviously both paths give "true." The satisfaction condition says this obviousness is guaranteed in general — and demanding that guarantee, for *every* signature morphism, is what makes a logic well-behaved.

## 4. Logics you didn't know were logics

The examples so far come from computer science, but nothing in the definition says "formal logic" in the narrow textbook sense. An institution is any statements-about-systems discipline that survives change of notation. Once you see the four-part pattern — vocabulary, statements, systems, satisfaction — it appears all over science and engineering. Run the disassembly on a few familiar fields:

**Differential equations.** A signature is a choice of state variables and parameters: position `x(t)`, velocity `v(t)`, mass `m`, spring constant `k`. Sentences are the equations themselves: `m·x″ + k·x = 0`. Models are candidate behaviors — smooth functions of time assigning a trajectory to each variable. Satisfaction is *solving*: a trajectory satisfies the sentence if substituting it in makes the equation hold identically. And the satisfaction condition is a habit so ingrained in physicists that it has become invisible: rename `x` to `q`, rescale to dimensionless variables, and the same motions solve the transformed equation. Truth invariant under change of notation. Reducts are familiar too — a full model of a coupled system, restricted to the vocabulary of one subsystem, is a model of that subsystem with the rest of the world forgotten.

**Process models and process calculi.** In formalisms like CCS, CSP, or the π-calculus, a signature is an alphabet of channels and actions; sentences are process expressions, or the properties we assert about processes ("every request is eventually acknowledged"); models are labeled transition systems — machines that actually exhibit behavior; satisfaction relates a machine to the expressions or properties it realizes. Signature morphisms are the everyday operations of process algebra — renaming and hiding of channels — and the satisfaction condition says observable behavior is stable under them. This is one of the cases that has been worked out rigorously as an institution in the literature, precisely so that process specifications can be combined, in one framework, with data-type specifications written in entirely different logics.

**Thermodynamics.** The vocabulary of a thermodynamic analysis is a choice of state variables (P, V, T, S, U, …) for a chosen system boundary. Sentences are constitutive relations and constraints: an equation of state like PV = nRT, the assertion that a process is adiabatic, an entropy inequality. Models are the physical systems themselves — more precisely, their manifolds of equilibrium states with a particular substance's properties. A model satisfies a sentence when the relation holds on it. Notice how much of the *craft* of thermodynamic reasoning is really signature engineering: choosing where to draw the system boundary and which variables to treat as independent is choosing a vocabulary, and deliberately ignoring internal degrees of freedom is taking a reduct.

**Quantum logic.** Birkhoff and von Neumann observed in 1936 that propositions about a quantum system ("the spin along z is up") obey a non-classical logic in which the distributive law fails. Institutionally: a signature selects the observables of interest; sentences are propositions about their values; models are quantum states; satisfaction says the state assigns the proposition probability one. The institutional view does not relitigate the foundations of quantum logic — its point is that quantum reasoning slots into the *same interface* as every other logic, so that translations between quantum and classical descriptions of a system become morphisms one can state and study rather than folklore.

**Statistical reasoning.** A statistician's signature is a choice of random variables and parameters. Sentences are modeling assertions and hypotheses: "the errors are independent and Gaussian," "θ > 0," "these two variables are conditionally independent given a third." Models are data-generating processes — actual probability distributions over outcomes. Satisfaction asks whether the process conforms to the assertion. This framing makes precise why thinking like a statistician differs from doing probability theory: probability theory is the mathematics *inside one model*, while statistical reasoning is the discipline of relating sentences to an unknown model — which is exactly the sentence/model interface that institutions abstract. One honest caveat: crisp true/false satisfaction fits sharp hypotheses, but much of statistical practice is *graded* — likelihoods, p-values, posteriors measure degrees of fit. This is not a dead end for the framework; Goguen and Burstall themselves defined *generalized institutions* in which satisfaction takes values beyond true and false, and graded fit walks in through precisely that door. (Part III returns to this door: graded evidence is exactly what a substrate for real science must carry.)

The moral of these examples: in institution theory, "a logic" means any disciplined interface between a vocabulary, the statements expressible in it, and the systems those statements describe. The formal logics of computer science are the cleanest instances, not the only ones. And this is what gives the translation machinery of Part II its reach — a morphism between institutions can, in principle, relate a process calculus to a differential-equation model of the same plant, or a quantum description to its classical control layer, with the satisfaction conditions on both sides guaranteeing that nothing is lost in translation.

## 5. Theories and specifications

Once you have sentences, you can bundle them. A **specification** (or presentation) is a signature together with a chosen set of sentences over it — the axioms. A **theory** is a specification closed under consequence: it contains not just the axioms but everything they entail. In our running example, the specification with signature `{0, s, +}` and the two addition equations above is (a fragment of) the specification of the natural numbers; its theory additionally contains derived facts like `∀n. s(0) + n = s(n)`.

The important structural fact is that specifications and theories inherit good behavior from signatures. If you can glue signatures together (technically: if the category of signatures has *pushouts*, which is the categorical name for "merge two vocabularies that share a common part, without accidental name clashes"), then you can glue theories together the same way. This single result is the mathematical foundation of **module systems** for specification and programming languages: importing a module, renaming its symbols, and combining modules that share a common submodule are all operations on theories, and institution theory shows they work in *any* logic satisfying the definition. Languages like OBJ, CASL, and CafeOBJ were designed directly on this foundation.

One refinement deserves mention because it matches engineering intuition so well. In practice, we rarely want to merely *map* one vocabulary into another — we want genuine *inclusions*, where a submodule uses literally the same symbols as the module containing it, so that shared symbols are automatically shared rather than laboriously matched up. **Inclusive institutions** add exactly this: a designated notion of inclusion among signatures and theories. With it, one can define modules with public and private parts — a Σ-interface exposed to clients and a larger Σ′ of hidden implementation symbols — again independently of the underlying logic.

## Where this leaves us

We now have the whole of a logic in four moving parts and one law: vocabularies that can change, sentences that ride those changes forward, models that ride them backward, and a truth relation that does not flinch when the notation does. We have seen the pattern is not parochial — it fits differential equations, thermodynamics, quantum propositions, and statistical inference as readily as it fits the algebra it was born from. And we have seen sentences bundle into theories and modules that compose in any logic at all.

Everything so far has happened *inside* a single logic. The interesting science — and the reason this framework matters for the coordination problems of Section 1 — begins when we relate two different logics to each other. That directional mismatch between sentences and models, flagged above as the most important structural fact in the subject, is about to earn its billing. [Part II — Translating Between Logics](/blog/translating-between-logics/) builds the translations between logics, shows the two ways they can run, and cashes them out as a concrete engineering dividend: reusing one logic's theorem prover inside another.

---

## Glossary for Part I

**Signature.** The vocabulary of non-logical symbols (sorts, functions, predicates) over which sentences are written.

**Signature morphism.** A translation between vocabularies: renaming, embedding, or more general mapping of symbols.

**Sentence.** A well-formed statement over a signature. Sentences translate *covariantly* (same direction as signature morphisms).

**Model.** An interpretation of a signature — the kind of structure sentences are true or false of. Models translate *contravariantly* (opposite direction), via reducts.

**Reduct.** The model obtained by viewing a model of a big vocabulary through a smaller one, forgetting the extra structure.

**Satisfaction condition.** The defining axiom of institutions: satisfaction is invariant under change of notation.

**Specification / presentation.** A signature plus a set of axioms.

**Theory.** A specification closed under logical consequence.

**Pushout.** The categorical operation of merging two structures along a shared part; the basis of module composition.

**Inclusive institution.** An institution whose signatures and theories carry a genuine notion of inclusion (literal sub-vocabularies), matching how modules share symbols in practice.

**Generalized institution.** A variant in which satisfaction takes values beyond true/false — the natural home for graded notions of fit such as likelihoods or degrees of satisfaction.

---

## Further reading

The canonical technical reference is Goguen and Burstall, *Institutions: Abstract Model Theory for Specification and Programming* (J. ACM, 1992). Diaconescu's book *Institution-Independent Model Theory* develops the mathematics in depth, and the CASL and Hets ecosystems show the ideas deployed at scale in working tools. Part II takes up the systematic theory of translations between logics, following Goguen and Roșu, *Institution Morphisms* (Formal Aspects of Computing, 2002).

