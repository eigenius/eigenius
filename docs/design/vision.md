# Eigenius: Vision and Technical Rationale

*April 2026*

---

## The Problem

The most consequential intellectual work — advancing quantum theory, designing drugs, engineering autonomous systems, building infrastructure that millions depend on — requires chains of reasoning that are long, interdependent, and unforgiving of error. A single flawed assumption in a quantum error correction proof invalidates everything downstream. A mischaracterized binding affinity in a drug candidate can waste years of clinical development. A structural miscalculation in a railway station can surface as a decade of cost overruns and engineering failures.

Large language models have demonstrated remarkable capability in scientific and engineering contexts: synthesizing literature, generating hypotheses, writing code, analyzing data. But they share a structural limitation that becomes dangerous as the stakes rise. An LLM produces text that reads like knowledge but carries no epistemic warranty. There is no structural mechanism to distinguish a correct derivation from a fluent hallucination. The output of a careful formal argument and the output of a plausible-sounding confabulation look identical: well-written prose with confident tone.

This is not a temporary limitation that will be resolved by scaling model parameters or improving training data. It is intrinsic to the paradigm. Probabilistic sequence prediction produces outputs distributed according to training statistics, not according to logical validity. A model can learn to produce text that *resembles* valid reasoning — and it does so remarkably well — but resemblance is not a guarantee. For research that pushes beyond the boundary of known science, or engineering that operates at the edge of human comprehension, resemblance is insufficient.

The consequence is a growing epistemic crisis in AI-assisted intellectual work. As LLMs become embedded in research and engineering workflows, the boundary between what has been verified and what merely sounds verified is dissolving. A literature synthesis produced by an LLM may contain accurate citations alongside fabricated ones, correct derivations alongside subtle errors, established results alongside hallucinated claims — and nothing in the output's structure reveals which is which. The consumer is left to verify everything manually, which defeats the purpose of using the model, or to trust the output probabilistically, which introduces uncontrolled risk into high-stakes decisions.

## Why Existing Approaches Fall Short

The current landscape of AI infrastructure does not address this problem. It routes around it.

Orchestration frameworks (LangChain, LlamaIndex, Semantic Kernel) focus on composing LLM calls into workflows. They manage prompt templates, retrieval pipelines, and tool integration. But they treat LLM outputs as opaque strings. There is no type system governing what flows between steps, no formal guarantee that a pipeline is well-formed, no mechanism to distinguish a verified result from an unverified one. The orchestration is structural, not epistemic.

Knowledge graph platforms (Neo4j, Amazon Neptune, Stardog) provide structured storage and querying for relational data. They can represent entities and their relationships with schema constraints. But they are disconnected from the reasoning process. A knowledge graph can store the conclusion of an analysis, but it cannot store the derivation that produced it in a way that is replayable, auditable, and queryable. The graph records *what is claimed*; it does not record *why it is believed* or *with what certainty*.

Retrieval-augmented generation (RAG) grounds LLM outputs in source documents, reducing hallucination by providing relevant context. This is valuable but architecturally limited. RAG systems retrieve passages; they do not verify that the model's use of those passages is logically valid. A model can correctly retrieve a paper and then mischaracterize its findings. The retrieval was grounded; the reasoning was not.

Formal verification systems (Lean, Coq, Isabelle) provide the strongest possible guarantees — machine-checked mathematical proof. But they exist in a separate universe from LLM-driven workflows. There is no infrastructure for connecting a formally proved result to the knowledge graph that contextualizes it, or for embedding formal verification as a step in an AI-driven analysis pipeline. Proof assistants verify theorems; they do not manage the broader epistemic context in which those theorems are used.

What is missing is a substrate that unifies these concerns: structured knowledge representation, typed processing pipelines, LLM integration, reasoning trace capture, and formal verification — all within a single, queryable, self-describing system that maintains epistemic distinctions as a first-class architectural concern.

## The Eigenius Architecture

Eigenius is an open-source platform for AI-driven science and engineering. Its core architectural commitment is that every piece of knowledge in the system — every fact, every derivation, every LLM interaction, every formal proof — is represented as a typed resource in a unified knowledge graph, with tracked provenance and queryable epistemic status.

### Typed Knowledge Graph

The foundation is Eigon, a canonical typed data format. Everything is a Resource — classes, properties, data types, formats, and instance data are all represented uniformly with IRI identity and typed property values. The core ontology is self-describing: Class is itself an instance of Class, creating a fixed point from which all other ontology definitions are derived. A three-layer type system separates primitive data types (determining JSON representation), format constraints (validating string values), and content types (declaring embedded content like Markdown or HTML). Domain knowledge, processing pipelines, capability registrations, reasoning traces, and proof terms are all expressed in Eigon. The knowledge graph is not a separate database that the system writes to; it *is* the system's representation of itself and the world.

This self-description is not an academic curiosity. It means the system can query its own structure with the same language it uses to query domain data. "What classes exist?" is the same kind of query as "what molecules bind to this receptor?" The meta-level and the object-level share a single query language (EigenQL), a single type system, and a single storage layer.

### Epistemic Categories

Every resource in the knowledge graph carries an epistemic status, computed from its provenance chain:

**Observed** — a recorded fact with external provenance. A measured binding affinity, a published experimental result, a sensor reading. The system does not vouch for its truth; it vouches for its provenance. "This is what was recorded, and here is where it came from."

**Derived** — a conclusion produced by a typed processing pipeline from other resources. The type system guarantees the pipeline was well-formed. The reasoning trace records every input, every step, every intermediate result. The derivation is replayable and auditable. "This follows from those inputs through this process." The derivation can be challenged and re-examined, but it is not formally proved.

**Verified** — a derived result that additionally carries a formal proof term, checked by a constructive type theory. The proof is a machine-checked certificate that the conclusion follows from the premises by the rules of the type theory. "This is mathematically certain given those axioms."

These categories are not labels applied by the user. They are computed from the resource's provenance graph and enforced by the system. A resource is verified if and only if it has a checked proof term. Transitions are monotonic — a derived result can be promoted to verified by attaching a proof, but a verified result cannot be silently downgraded. The system always tells you the epistemic status of any conclusion, and the status is always grounded in the actual provenance, not in a claim.

### Typed Processing Pipelines

Processing pipelines in Eigenius are not scripts or prompt chains. They are programs in a typed functional language, validated by a dependent type theory (EigenTT, a fragment of the Calculus of Inductive Constructions — the same theory that underlies Lean 4). A pipeline that passes type-checking carries a formal guarantee: its bindings are type-compatible, its required inputs are provably available, and its control flow terminates. This is static verification before execution begins.

The type system supports partial evaluation through Normalization by Evaluation (NbE). Given a pipeline and a subset of its inputs, the system produces a well-typed residual — a simplified pipeline that awaits only the remaining inputs. This is not an optimization heuristic; it is a theorem about the type system, and it is a proof target in the formal specification track.

Every pipeline step produces typed output resources that enter the knowledge graph with full reasoning traces. An LLM call within a pipeline is not a black box — it is a typed Component with declared input and output classes, and its invocation is recorded as a reasoning trace capturing the prompt, the response, token usage, latency, and the provider configuration. The LLM's contribution is preserved as an auditable record, not dissolved into the final output.

### Formal Verification as a Capability

The architecture does not treat formal verification as a separate, privileged operation. It treats it as an instance of the general capability protocol — a registered service that the system can dispatch to when it encounters a resource that requires proof checking. Lean 4, Coq, Agda, or any constructive type theory can be registered as a capability. When a derivation carries a proof term, the system dispatches it to the appropriate proof checker, and the result (verified or rejected) becomes a typed resource in the knowledge graph.

This means formal verification deepens incrementally. A research team can begin by recording observations and running LLM-assisted analyses (observed and derived knowledge). As specific results become important enough to warrant formal treatment, proofs can be constructed and attached. The epistemic status of each conclusion is always visible, and the system explicitly shows where the boundary between derived and verified knowledge lies.

The Rust kernel is designed from the outset to have a provable correspondence with a Lean 4 formal specification. Critical kernel algorithms carry Verus proof annotations. The dependent type theory used for pipeline validation is a direct fragment of CIC — Lean 4's own core theory — so the formal specification is a scaled-up version of the same computational model, not a translation into a different formalism. This alignment is deliberate: it means the path from "validated by the type checker" to "proved in Lean 4" is a matter of degree, not kind.

## Where This Matters

### Frontier Physics

Quantum physics and quantum gravity operate at the boundary of human mathematical ability. A derivation in quantum error correction might involve hundreds of steps across linear algebra, probability theory, and information theory. An LLM can assist with literature synthesis, hypothesis generation, and even proof sketch construction. But the question "is this derivation correct?" requires more than plausible-sounding text. It requires verification.

Eigenius provides the infrastructure for a workflow where an LLM-assisted analysis produces a derived result, the derivation is recorded as a typed, replayable pipeline with full reasoning traces, and the critical steps are progressively formalized until the conclusion carries a machine-checked proof. At every stage, the researcher knows exactly which parts are verified, which are derived but unverified, and which are observational. The system does not pretend that LLM-assisted reasoning is the same as formal proof. It makes the gap explicit and provides a path to close it.

### Life Sciences and Drug Discovery

Drug discovery involves chains of inference that span molecular biology, chemistry, pharmacology, and clinical medicine. A candidate molecule's viability depends on binding affinity predictions, ADMET property estimates, pathway analyses, and toxicity assessments — each drawing on different models, databases, and experimental results, each with different uncertainty profiles.

Current AI-assisted drug discovery tools produce predictions. Eigenius would produce predictions *with provenance*: which experimental data grounded the binding affinity estimate? Which computational model produced the ADMET prediction, and what were its training set limitations? Which literature claims support the pathway hypothesis, and have those claims been independently replicated? The knowledge graph makes these questions answerable — not as manual literature review, but as typed queries over the system's own reasoning history.

When a pharmaceutical team reviews a candidate, the question is not just "what does the model predict?" but "why does it predict this, what assumptions is the prediction based on, and which of those assumptions have been independently verified?" Eigenius makes these questions structural rather than rhetorical.

### Autonomous Humanoid Systems

Building an autonomous humanoid robot that operates in the real world integrates mechanical engineering, control theory, computer vision, natural language understanding, planning, and real-time decision-making. Each domain has its own models, its own uncertainty profiles, and its own verification standards. The interactions between domains create emergent complexity that no single specialist fully understands.

The challenge is not just building each subsystem but maintaining coherence across them: does the control system's model of the robot's kinematics match the mechanical design? Do the vision system's assumptions about the environment match the planner's? Does the safety system's analysis account for all failure modes that the mechanical and electrical designs introduce?

These cross-domain coherence questions are precisely what a typed knowledge graph with formal verification support is designed to answer. The kinematic model, the control parameters, the safety constraints, and the environmental assumptions can all be represented as typed resources with explicit dependencies. Inconsistencies become type errors. Unverified assumptions become visible gaps in the epistemic chain.

### Complex Civic Infrastructure

The construction of Berlin Brandenburg Airport (BER) — delayed by nine years, exceeding its budget by billions — is a case study in what happens when a project's complexity exceeds human ability to track dependencies, verify specifications, and maintain consistency across thousands of interacting systems. The Stuttgart 21 railway station project exhibits similar patterns: cascading delays driven by specification changes whose downstream effects were not fully traced.

These are not primarily failures of engineering competence. They are failures of epistemic infrastructure. The knowledge required to build a major railway station or airport exists — in engineering specifications, regulatory codes, simulation results, test reports, and expert judgment — but it exists in fragments scattered across thousands of documents, models, and databases, with no unified mechanism to query across them, trace dependencies, or verify consistency.

A typed knowledge graph that represents structural specifications, regulatory requirements, simulation results, and construction constraints as typed, queryable resources — with formal verification of critical load-bearing calculations — would make dependency tracing and consistency checking structural operations rather than manual review processes. When a specification changes, the system can answer: "what else depends on this specification, and which downstream analyses need revalidation?" This is a query, not a meeting.

## Design Principles

Several principles guide the architecture. They are not aspirational statements; they are constraints that the implementation enforces.

**Epistemic honesty over convenience.** The system never conflates epistemic categories. A derived result is never silently presented as verified. An LLM output is never treated as equivalent to a formal proof. The system maintains these distinctions even when collapsing them would be more convenient for the user. The inconvenience of seeing "derived, unverified" next to a result is the system working correctly.

**Provenance is not optional.** Every resource in the knowledge graph has a provenance chain. There is no mechanism for inserting knowledge without recording where it came from. This is enforced at the type system level, not as a policy guideline. The cost is storage and complexity. The benefit is that "where did this come from?" is always answerable.

**Formal verification deepens incrementally.** The system is useful before any formal proofs exist. Recording observations and running typed pipelines with reasoning traces is valuable on its own. Formal verification is an additional layer that can be applied selectively to the most critical conclusions. The architecture does not demand that everything be proved; it demands that the system always show you what *has* been proved and what has not.

**Self-description as a structural commitment.** The system describes itself using its own primitives. Processing pipelines, capability registrations, reasoning traces, and proof terms are all Eigon resources in the knowledge graph. This is not a convenience feature; it is the mechanism that makes the system coherently queryable. "What capabilities are registered?" and "what reasoning traces reference this assumption?" are both EigenQL queries.

**Open architecture.** The platform is open source. The kernel's formal properties are specified in Lean 4 and are publicly verifiable. The capability protocol allows domain-specific extensions without modifying the kernel. The storage layer is an abstract interface that admits multiple backends. The system does not create vendor lock-in; it creates an open substrate that research communities can build on, extend, and verify.

## Technical Foundation

The architecture comprises a native Rust kernel with formal proof annotations (Verus), a Deno/TypeScript orchestration layer for pipeline execution and LLM integration, and TiKV as the distributed storage engine. The kernel exposes a gRPC service API; the orchestration layer is an API client, not a runtime host. Untrusted domain extensions run in WASM sandboxes managed by the kernel via Wasmtime.

The type system is founded on EigenTT, a minimal dependent type theory that is a direct fragment of CIC — the Calculus of Inductive Constructions that underlies Lean 4. This alignment is architectural, not incidental: it means the pipeline type checker and the formal verification track operate on the same mathematical foundation, and the path from type-checked to formally proved is continuous rather than requiring a translation between formalisms.

EigenQL, the query language, begins as a typed conjunctive query language (v1) with a documented extension path to recursive Datalog. The v1 language is computationally well-behaved (guaranteed termination, monotonic with respect to the knowledge graph) while remaining expressive enough for the pattern matching and join operations that scientific knowledge graphs require.

The full architecture specification, implementation plan, and design document requirements are maintained in the project repository.

## What Success Looks Like

Eigenius succeeds if it changes how researchers and engineers relate to AI-assisted conclusions. Specifically:

A researcher using Eigenius to analyze quantum error correction codes can query: "show me all conclusions in this analysis that are formally verified, and show me the unverified assumptions they depend on." The system answers with a typed, complete response — not a summary, not a best guess, but a structural accounting of what has been proved and what has not.

A pharmaceutical team reviewing a drug candidate can query: "what experimental evidence supports this binding affinity prediction, and what is the provenance chain from raw data to conclusion?" The system traces the chain through every computational step, every model invocation, every data source, and presents it as an auditable record.

An engineering team managing a complex infrastructure project can query: "if we change this structural specification, what downstream analyses and certifications are affected?" The system answers by traversing the typed dependency graph, identifying every resource that directly or transitively depends on the changed specification.

These are not hypothetical capabilities. They are direct consequences of the architectural decisions: typed resources, tracked provenance, replayable derivations, formal verification as a capability, and a unified query language across all epistemic levels.

The ambition is not to replace human judgment. It is to give human judgment the infrastructure it needs to operate reliably at the scale of problems that now exceed unaided human comprehension.

---

*Eigenius is open source under the Apache 2.0 license. The architecture specification, implementation plan, and formal specifications are maintained at [github.com/eigenius](https://github.com/eigenius).*
