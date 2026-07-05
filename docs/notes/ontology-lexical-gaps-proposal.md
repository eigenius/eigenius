Proposal: Resolving Ontological Gaps and Metonymy in Biomedical Combinatorial Parsing

Subject: Architectural handling of Gene Families, Multi-Word Expressions, and Morphological Functors (The "RecQ Phenomenon")
Context: Combinatorial Grammar Engine with Dependent Type Semantics ($\Sigma$-Types) Augmented by Large Language Models (LLMs)

1. Executive Summary

Biomedical literature parsing frequently stalls at the boundaries of rigid ontologies (like UMLS and NCBI Gene). Authors use multi-word expressions (e.g., "RecQ DNA helicase"), utilize derivational morphology to express homology (e.g., "RECQ-like"), and metonymically conflate gene families with specific protein products.

This proposal outlines a hybrid Neuro-Symbolic strategy. At its core, we resolve linguistic phenomena by leveraging our engine's dependent type architecture (modeling ontological classes as $\Sigma$-types). To ensure lexical robustness and scalability, this logical core is augmented by an LLM and retrieval (RAG) layer, which dynamically builds custom glossaries, adjusts complex language, and maps unstructured text to formal semantics on the fly.

2. Problem Statement: The "RecQ" Phenomenon

The extraction of relations involving "RecQ" exposes three distinct limitations in standard lexical approaches:

Lexical Voids: "RecQ DNA Helicase" does not exist as a contiguous string in UMLS or NCBI Gene, despite being semantically equivalent to UMLS C0084304 (RecQ Helicases).

Class/Instance Conflation: "RecQ" is a gene family. In texts, it is frequently used as a proxy for its members (WRN, BLM, RECQL) or treated as a singular physical protein taking part in physical interactions (e.g., "RecQ binds to...").

Productive Morphology: The suffix "-like" (as in "RECQ-like") dynamically generates a new ontological class based on sequence homology or functional similarity, requiring on-the-fly type construction.

3. Theoretical Framework: Classes as $\Sigma$-Types

Our engine models ontological classes not as atomic labels, but as dependent pairs ($\Sigma$-types), where an entity is paired with the proof of its properties:

$$\text{Class}_{A} = \Sigma_{(x : \text{Entity})} P(x)$$

This constructive approach means that parsing a noun phrase produces a dependent record. This provides the mathematical foundation for natural subtyping via projection ($\pi_1$) and intersection via extended tuples.

4. Proposed Architectural Solutions (Logical Layer)

4.1. Compositional Derivation of Complex Nominals

Instead of relying strictly on pre-computed Multi-Word Expression (MWE) dictionaries, the grammar will dynamically construct missing concepts like "RecQ DNA helicase" through compositional application.

Lexical Typing:

$\text{DNA helicase} \vdash N : \Sigma_{(h : \text{Protein})} \text{Function}(h, \text{UnwindDNA})$

$\text{RecQ} \vdash N/N : \lambda C. \Sigma_{(c : C)} \text{BelongsToFamily}(c, \text{RecQ\_Family})$

Combinatorial Result:

$\text{RecQ DNA helicase} \vdash N : \Sigma_{(h : \text{Protein})} (\text{Function}(h, \text{UnwindDNA}) \times \text{BelongsToFamily}(h, \text{RecQ\_Family}))$

4.2. Resolving Metonymy via Projection ($\pi_1$)

When a text states "RecQ binds to DNA", the verb "binds" strictly expects a physical Protein as an argument, but "RecQ" represents a GeneFamily class. Using $\Sigma$-types, we define a specific member of a family as:

$$\text{FamilyMember}(F) = \Sigma_{(p : \text{Protein})} \text{BelongsTo}(p, F)$$

When the parser encounters a type mismatch (Verb expecting Protein, Subject yielding FamilyMember(RecQ)), the type system will automatically apply the first projection $\pi_1$.

$x : \text{FamilyMember}(\text{RecQ})$

$\pi_1(x) : \text{Protein}$

This justifies the metonymy mathematically, retaining the semantic knowledge that the protein belongs to the RecQ family.

4.3. Typing Derivational Functors ("-like")

To handle productive morphology like "RECQ-like", we treat the suffix "-like" as a morpho-syntactic functor:

Syntactic Category: $(N/N)\backslash N_{family}$

Semantic Type: $\lambda F. \lambda X. \Sigma_{(x : X)} \text{HomologousTo}(x, F)$

5. LLM and Retrieval Augmentation (Neuro-Symbolic Integration)

While $\Sigma$-types provide logical rigor, depending entirely on a static lexicon is brittle. To make the system "sufficient" and highly scalable, we will integrate an LLM pipeline to act as a dynamic frontend for the grammar engine.

5.1. Dynamic Predicate Grounding via RAG

When the parser encounters an Out-Of-Vocabulary (OOV) multi-word expression, it will trigger a Retrieval-Augmented Generation (RAG) pipeline:

Retrieve: Search UMLS, NCBI, and internal definitions for the OOV term.

Synthesize: An LLM processes the retrieved text and outputs a provisional formal $\Sigma$-type.

Parse: The grammar engine uses this LLM-generated type to complete the parse. This bridges the gap between unstructured text definitions and strict mathematical types.

5.2. Custom Glossary Bootstrapping

We will use an LLM to pre-process large corpora to dynamically build the custom vocabulary. The LLM will identify novel entities, acronyms, and gene families, automatically proposing base syntactic categories (e.g., tagging a newly discovered family as $N/N$ modifier) to feed the grammar engine's base lexicon.

5.3. Syntactic Normalization (Language Adjustment)

Biomedical literature often features highly irregular, convoluted syntax that causes combinatorial explosion or failure in strict parsing. The LLM will be used as a pre-processing filter to "adjust" or normalize language into canonical forms while preserving meaning. For example, rewriting passive, nested clauses into active, direct relations that the CCG parser can easily and deterministically resolve.

6. Implementation Roadmap

Phase 1: Foundation (Logic)

Introduce modifier types ($N/N$) for recognized family names.

Implement the morphological functor for "-like" and implicit $\pi_1$ coercion.

Phase 2: RAG and Glossary API

Connect the parsing engine to a vector database containing UMLS and NCBI definitions.

Implement the LLM API to translate retrieved definitions into draft $\Sigma$-type signatures.

Phase 3: Pipeline Integration

Deploy the LLM syntactic normalization step prior to CCG parsing.

Establish a feedback loop where successful LLM-assisted parses are cached into the permanent custom glossary.

7. Conclusion

By treating ontological classes as $\Sigma$-types, the combinatorial grammar engine solves the structural complexities of metonymy and homology. By augmenting this rigorous logical core with an LLM and retrieval-based framework, the engine gains the statistical flexibility required to handle lexical gaps, garbled syntax, and the ever-expanding biomedical vocabulary. This Neuro-Symbolic approach ensures both high-fidelity semantic representation and robust real-world scalability.