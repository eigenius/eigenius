---
title: "Part II — Translating Between Logics"
subtitle: "Institutions, Part II — Morphisms, Comorphisms, and the Art of Borrowing"
description: "Morphisms, comorphisms, and borrowing. What it means to move a statement from one logic into another without changing what it claims — and why heterogeneous modelling needs category theory to stay honest."
pubDate: 2026-07-23
author: "Eigenius"
tags: ["institutions", "logic", "comorphisms"]
draft: false
series:
  name: "Institutions"
  part: 2
---

*The second of three parts on the theory of institutions. [Part I](/blog/what-is-a-logic/) disassembled a single logic into four parts — vocabulary, sentences, models, satisfaction — held together by one law: truth is invariant under change of notation. This part relates* different *logics to one another, and turns those relations into a concrete payoff: reusing one logic's theorem prover inside another. Part III makes the whole apparatus machine-checkable and applies it to AI-assisted science.*

---

## Recap: the four parts and the one law

Before we connect logics, a one-paragraph reminder of what a logic *is*, in this framework. An **institution** packages a logic as: a collection of **signatures** (vocabularies) with **signature morphisms** (vocabulary changes) between them; for each signature, a set of **sentences** that translate *forward* along a vocabulary change; for each signature, a collection of **models** that translate *backward* (the *reduct*); and a **satisfaction** relation `M ⊨ φ` obeying the **satisfaction condition** — translate the sentence forward or reduce the model backward, and the truth value agrees. The one fact to keep in the front of your mind, because this entire part hinges on it: **sentences flow forward, models flow backward.** That mismatch is not a wrinkle to be ironed out; it is the engine of everything below.

## 6. Translating between logics: morphisms and comorphisms

In Part I, everything happened inside one logic. The real fun begins when we relate two logics to each other. What should a "translation between institutions" be? It needs three parts, one for each layer: a mapping of signatures, a mapping of sentences, and a mapping of models — plus a compatibility requirement echoing the satisfaction condition. The subtlety, once again, is *direction*.

Consider our running pair from Part I: equational logic (EQ) and first-order logic with equality (FOL=). A FOL= signature is richer — it has predicate symbols in addition to function symbols. There are two natural ways to relate the logics, and they run in opposite directions.

**The forgetful direction (an institution morphism).** Map each FOL= signature down to an EQ signature by *forgetting* the predicate symbols. Each FOL= model forgets down to an algebra the same way. But sentences go the other way: every equation over the stripped-down signature can be *read as* a first-order sentence over the full one. So: signatures and models travel from the richer logic to the simpler one; sentences travel from the simpler to the richer. This package — signatures and models forward, sentences backward — is an **institution morphism**, the notion Goguen and Burstall introduced originally. It expresses "FOL= has at least the expressive structure of EQ inside it; here is how to project it out."

**The embedding direction (an institution comorphism).** Alternatively, map each EQ signature *up* into FOL= by pairing it with an empty set of predicates. Every equation embeds as a first-order sentence, traveling the same direction as signatures. Models come back down: a first-order model of the embedded signature is, in particular, an algebra. This package — signatures and sentences forward, models backward — is an **institution comorphism** (elsewhere in the literature: "plain map" or "representation"; the profusion of names for the same idea is a known hazard of this field). It expresses "EQ can be encoded inside FOL=."

These two notions are precisely dual, and when the signature translations are adjoint to each other — loosely, when "forget" and "freely embed" are two sides of the same coin, as they are here — morphisms and comorphisms correspond exactly, so you can convert one description into the other.

Which should you prefer? Goguen and Roșu argue, with examples, that the **forgetful (morphism) formulation is usually simpler and more natural**, for the same reason that the forgetful functor from groups to sets describes the group/set relationship better than the free-group construction does: forgetting involves no arbitrary choices, while embedding always requires inventing something (an empty predicate set, a default element, an error value). A useful rule of thumb when you meet a new inter-logic translation: first ask what gets *forgotten*, and only then ask what gets *encoded*. (This proverb returns in Part III with surprising force: even the relationship between explicit evidence and mere provability turns out to be a forgetful morphism.)

## 7. The payoff: borrowing theorem provers

Here is the concrete engineering dividend. Suppose you work in logic L, which has no good automated prover, but there is an excellent prover for logic L′, and you have a translation between them satisfying a modest extra condition (roughly: every model on the L side is hit by the model translation). Then a **borrowing theorem** holds: a sentence follows from your axioms in L *if and only if* its translation follows from the translated axioms in L′. You can ship your proof obligations across the translation, run the foreign prover, and trust the verdict back home.

This is not hypothetical. Encoding equational problems into first-order provers, first-order problems into higher-order proof assistants, and the many logics of the CASL ecosystem into each other are all engineered as institution (co)morphisms, precisely so that borrowing theorems apply. The translation does the bookkeeping — for instance, when moving from FOL= to FOL without built-in equality, the translation must add the axioms making `=` an equivalence relation compatible with all operations — and the theory tells you exactly which bookkeeping suffices.

Borrowing is one dividend; **heterogeneous modeling** is the other. When a single system spans several logics — the cyber-physical, quantum, and data-pipeline examples from Part I — a network of institutions connected by morphisms gives the whole model a joint semantics: each component lives in its native logic, and the morphisms say precisely how a statement or a model in one component constrains the others. Tools like Hets implement exactly this picture, a graph of logics with translations as edges. The alternative — informal hand-offs between the discrete and continuous halves of a design, or between the statistical model and the database schema — is where interface bugs breed. Part III revisits this graph-of-logics picture in the age of proof assistants and AI authorship, where "trust the verdict" becomes "check the certificate" and the graph becomes the substrate an entire scientific record can be built on.

## 8. A field guide to the morphism zoo

The literature contains a bewildering variety of translation notions with inconsistent names. The following table summarizes the main species in the Goguen–Roșu nomenclature. "Sig," "Sen," "Mod" indicate which way each component travels relative to the direction of the arrow between institutions.

| Name | Sig | Sen | Mod | Intuition | Also known as |
|---|---|---|---|---|---|
| **Morphism** | → | ← | → | Project richer logic onto simpler one; forgetful | (original notion) |
| **Comorphism** | → | → | ← | Encode simpler logic inside richer one | plain map, representation |
| **Theoroidal morphism / comorphism** | maps theories, not just signatures | | | Translation needs axioms on the target side (like importing a library, not just a namespace) | maps of institutions (comorphism case) |
| **Simple theoroidal** | signatures ↦ theories | | | Each vocabulary translates to a vocabulary *plus standard axioms* (e.g., equality axioms) | simple maps |
| **Forward morphism** | → | → | → | Everything travels together; e.g., re-expressing a logic in an enriched setting | backward comorphism |
| **Semi-natural (co)morphism** | as above, but the model translation need not be uniform across signature changes | | | For constructions like free completions that are natural per-signature but not globally | related to general maps |

Two practical notes. First, "simple" theoroidal morphisms are the workhorse of borrowing in practice — "translate my signature and throw in the standard axioms" is exactly what encoding equality, partiality, or subsorting into a plainer logic looks like. Second, when in doubt about which species you have, chase the direction of each of the three components; the name follows mechanically.

## 9. What you need from category theory (and what you don't)

Institutions are stated in categorical language, but the entry cost is lower than the papers make it look. To read and use the definitions in these articles, you need exactly three concepts: a **category** (objects and composable arrows — signatures and signature morphisms form one), a **functor** (a structure-respecting mapping between categories — the sentence assignment Sen and the model assignment Mod are functors), and a **natural transformation** (a family of mappings that is uniform across a whole category — the sentence and model translations of a morphism are these). Any first chapter of any category theory text covers all three.

What you do *not* need, unless you intend to prove new general theorems: indexed categories, Kan extensions, comma categories, and the completeness/cocompleteness machinery. Those tools exist so that results like "the category of institutions has all limits" can be proved once, abstractly, instead of by hand — they are the *how* of the metatheory, not the *what* of the concept. It is entirely respectable to use institutions the way most engineers use linear algebra: fluently, and without re-deriving the spectral theorem.

## Where this leaves us

We can now do more than describe individual logics: we can wire them together. A **morphism** projects a richer logic onto a simpler one by forgetting; a **comorphism** encodes a simpler logic inside a richer one; the two are dual, and the forgetful direction is usually the more natural. Wiring in hand, two dividends follow. **Borrowing** lets a logic with no prover of its own lean on a neighbor's, with a theorem certifying that the borrowed verdict is binding at home. And **heterogeneous modeling** gives a multi-logic system a single joint semantics — a graph of logics with verified translations as its edges.

But notice the quiet assumption in the phrase "trust the verdict." A borrowing theorem tells you the foreign prover's yes-or-no answer is *correct*; it does not hand you a checkable object. In an age when a fast, fallible AI may be the thing producing those answers, "trust the verdict" is exactly the assumption we can no longer afford. [Part III — Making Reasoning Checkable](/blog/making-reasoning-checkable/) upgrades it. By moving institutions inside a constructive proof assistant, satisfaction stops being a bare yes-or-no and starts carrying a *witness* — evidence a machine can inspect. Borrowing becomes a compiler of certificates; the graph of logics becomes a substrate in which a scientific argument is not an essay to be reviewed but an object to be checked. And that is where the [Eigenius](https://eigenius.io/) project enters.

---

## Glossary for Part II

**Institution morphism.** A translation between logics with signatures and models going forward and sentences going backward; typically forgetful.

**Institution comorphism.** The dual: signatures and sentences forward, models backward; typically an encoding.

**Duality of morphisms and comorphisms.** When the underlying signature translations are adjoint, morphisms and comorphisms correspond exactly and either can be converted into the other.

**Borrowing.** Reusing a theorem prover for one logic to settle proof obligations in another, justified by a (co)morphism with a suitable surjectivity property.

**Heterogeneous modeling.** Giving a system that spans several logics a single joint semantics via a network of institutions connected by (co)morphisms.

**Theoroidal.** Prefix meaning the translation targets theories (vocabularies with axioms attached) rather than bare signatures.

**Simple (theoroidal) map.** A translation sending each signature to a theory — a vocabulary plus standard axioms; the practical workhorse of borrowing.

**Category / functor / natural transformation.** The three categorical notions actually needed to read institution theory: objects-with-arrows, structure-preserving maps between them, and uniform families of such maps.

---

## Further reading

The systematic treatment of the translation zoo, whose terminology this part follows, is Goguen and Roșu, *Institution Morphisms* (Formal Aspects of Computing, 2002). For borrowing, see Cerioli and Meseguer, *May I Borrow Your Logic? (Transporting Logical Structures along Maps)* (Theoretical Computer Science, 1997). The canonical foundation remains Goguen and Burstall, *Institutions: Abstract Model Theory for Specification and Programming* (J. ACM, 1992); the Hets toolkit demonstrates the graph-of-logics picture in working software. Part III turns to the constructive, proof-relevant reading, following Reynolds and Monahan, *Reasoning about Logical Systems in the Coq Proof Assistant* (Science of Computer Programming, 2024).
