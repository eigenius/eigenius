# Eigenius 2026–2027 Publication Roadmap

*Planning document · May 2026*

Outlines for the three highest-leverage papers to publish first, ordered by recommended sequencing. Each is sized for its target venue, has a clear contribution-vs-related-work delta, and has been picked to establish credibility in a specific subfield without overlapping the others.

The strategic logic: paper 1 establishes that the platform exists and what it stands for; paper 2 establishes the deepest formal contribution in its natural community; paper 3 demonstrates the platform solving a problem that the broader CS community is actively wrestling with in 2026.

A note on the third slot: this roadmap puts the software-engineering-as-knowledge-graph paper there because it rides the agentic-coding moment, has D35 already as a substantive design basis, and reaches the broadest audience. The leading alternative would be a layer-aware ANN search paper (VLDB/SIGMOD) — more measurable and a hotter database-systems topic, but narrower in reach and requiring substantial implementation work before submission. If the implementation is further along than the SE dogfooding effort by mid-2026, swap them.

---

## Paper 1 — Eigenius: An Epistemically-Typed Knowledge Graph for AI-Driven Science and Engineering

**Venue.** CIDR 2027. Submissions typically September 2026; conference January 2027. CIDR is the right home: vision papers welcomed, the audience spans systems and AI, and the format suits a paper whose contribution is a unified architecture rather than a measured speedup.

**Backup venue.** CACM as a Practice piece (longer lead time, broader audience, less prestige in the database community). Or VLDB Vision Track (deadline March; conference August/September) if the CIDR window slips.

**Length.** 10–12 pages CIDR format.

**Single-sentence contribution.** Eigenius unifies typed knowledge representation, dependently-typed processing pipelines, formal verification, and reasoning-trace capture in a single self-describing system whose architectural commitment is that every resource carries computed epistemic status — a design that no existing knowledge-graph or AI-orchestration platform makes.

**Section outline.**

*1. Introduction — the epistemic crisis (1.5 pages).* Frame the problem: AI-assisted reasoning produces fluent text without epistemic guarantees. Knowledge graphs (Neo4j, Stardog, Neptune) store claims without status. LLM orchestration frameworks treat outputs as opaque strings. Formal verification systems live in a separate universe from the working pipelines that actually need them. The gap is structural, not incremental: no existing system unifies these concerns. Position paper as an architectural answer, not a performance result.

*2. Eigenius in one figure (1 page).* High-level diagram: kernel, orchestration, institutions, capability protocol, layer chain. Six-paragraph summary of what each does. The reader should leave §2 knowing the shape of the system; §3 onward develops the load-bearing pieces.

*3. Epistemic categories as a database primitive (2 pages).* The four categories: Declared, Observed, Derived, Verified. Status is *computed from* provenance, not asserted by the user. Monotonic promotion, no silent downgrade. Distinct from W3C PROV-style lineage (which records origin without certification level) and from traditional database constraints (which validate well-formedness, not epistemic standing). Worked example: the same molecular-binding-affinity claim as Observed (measured), Derived (model-predicted), and Verified (proof-checked under stated axioms) — three resources, three statuses, all queryable through the same surface.

*4. Resources as sentences (1.5 pages).* The Diaconescu-style insight: resources are *typed claims* in an institution's logic, not interpretations or models. This is the architectural move that lets the system carry verified, derived, and observed knowledge in a single store without conflating their meaning. Implications: the kernel doesn't need to "know" what each domain's model means, only that the institution can extract sentences, reify them, and check satisfaction.

*5. Mini-TT and the Lean 4 alignment (1.5 pages).* The processing-pipeline type system is a fragment of the Calculus of Inductive Constructions — same theory as Lean 4. Pipelines are dependent functions; type-checking guarantees well-formedness and termination. NbE for partial evaluation produces well-typed residuals. The continuous path from type-checked to formally proved is architectural, not a translation between formalisms. Why this matters for the broader argument: the gap between "runtime-validated" and "formally verified" is one of degree, not kind.

*6. Institutions as the extension mechanism (1 page).* Goguen-Burstall realised in code. Three-method trait, QueryClass with dispatch_role (OnDemand / AutoOnLoad / Decidable), Verdict-gated Loads. The deeper paper-2 contribution lives at CALCO; here only the operational consequences: extension without kernel modification, verdict-gated state transitions, and the deliberate non-composition of comorphisms.

*7. Self-description (1 page).* `Class` is itself an instance of `Class`. The ontology, programs, traces, capability registrations, and reasoning history are all Eigon resources, queryable through one language. Meta-level and object-level share a single store. The practical consequence: "what capabilities are registered?" and "what reasoning traces reference this assumption?" are both EigenQL queries against the same kernel.

*8. Applications and lessons learned (1.5 pages).* Brief survey: drug discovery (provenance of binding-affinity predictions), quantum verification (mechanically-checked steps in error-correction derivations), civic infrastructure (dependency tracing across thousands of specifications), software engineering (the D35 case study; forward-reference paper 3). Honest section on what we got wrong — vision papers benefit from intellectual humility. Two examples of design decisions that didn't survive contact with implementation, and what we learned.

*9. Related work (1 page).* Knowledge graphs and provenance systems (Neo4j, Stardog, Neptune; W3C PROV, Trio, Perm, ProvSQL — the distinction between lineage and epistemic status is the key delta). Type-theoretic frameworks (Lean 4, Coq, Agda, Verus — alignment with vs. integration into). LLM orchestration (LangChain, LlamaIndex, DSPy — what they don't model). Institution theory (Goguen, Burstall, Diaconescu — Eigenius as a working instance, with the deeper treatment deferred to paper 2).

*10. Discussion and conclusion (0.5 pages).* Open questions: scale-out beyond TiKV, embedding-model-versioning under content-addressing, the human-authoring UX for ontology evolution. Closing claim: epistemic infrastructure is not a feature of AI systems but a precondition for trusting them in serious work.

**Related-work delta to make crisp.** Against PROV: PROV records *where claims came from*; epistemic categories record *what level of certification claims have*. These are orthogonal, and Eigenius implements both. Against typed databases: type systems validate well-formedness; epistemic categories validate epistemic standing — a well-formed Derived claim and a well-formed Verified claim differ in nothing structural except their provenance graph. Against neuro-symbolic systems: Eigenius does not impose symbolic reasoning over LLM outputs; it preserves both as typed evidence with status.

**Existing assets to draw on.** `papers/eigenius-early.tex` and `papers/eigenius-institutions.tex` are the obvious starting points; `design/vision.md` and `design/manifesto.md` carry the rhetorical framing already.

---

## Paper 2 — Realising Institutions: A Working Architecture for Fibred Knowledge Graphs

**Venue.** CALCO 2027. Biennial conference (CALCO 2025 was the most recent); 2027 timing aligns. Submission typically March/April 2027; conference June/July. The audience is exactly the institution-theory community — Diaconescu would be a plausible reviewer or even a co-author if the relationship can be established.

**Backup venues.** WADT (the IFIP WG 1.3 working conference, more workshop-shaped); LICS (more general theoretical CS, less direct fit); the Journal of Logical and Algebraic Methods in Programming for an extended journal version.

**Length.** 16–20 pages CALCO format (LNCS).

**Single-sentence contribution.** This paper presents the first production realisation of Goguen-Burstall institution theory as the extension mechanism of a working knowledge graph platform, including the engineering decisions, the design surprises, and a practical analysis of why comorphism non-composition (Diaconescu's Fact 14.9) is the right registry-level commitment.

**Section outline.**

*1. Introduction (1.5 pages).* Institution theory is 35+ years old and widely cited as the foundation for heterogeneous formal specification. Production realisations are surprisingly thin: Hets is the canonical example, focused on specification languages; CafeOBJ and Maude are language-internal. Eigenius is the first attempt — to our knowledge — to use institutions as the *runtime extension surface* of a knowledge graph platform, with concrete instantiations including LLM extraction, theorem provers (Lean 4 via D28), domain-specific solvers (Julia institutions via D27), and validation institutions for software engineering (D35). The paper is part architectural description, part design analysis, part reflection on what the theory predicted correctly and what it didn't cover.

*2. Background (2 pages).* Concise recap of institution theory — signature category, sentence functor, model functor, satisfaction relation. Comorphisms, morphisms, theoroidal comorphisms, Diaconescu's recent work on heterogeneous specification. Just enough background that a reader from the broader formal-methods community can follow §3 onward without prior CALCO exposure. Cite the standard treatments (Sannella & Tarlecki *Foundations of Algebraic Specification*; Diaconescu *Institution-independent Model Theory*).

*3. The base category (2 pages).* Eigon as the shared signature category — emphatically *not* itself an institution, but the carrier in which all institutions' sentences are encoded. The three-layer type system (primitive, format, content) and how it relates to the signature categorical structure. Resources-as-sentences, with the typed-property structure as the syntactic shape. This is a load-bearing decision worth defending: the temptation is to make Eigon an institution; the right move is to keep it neutral.

*4. Institutions as a trait (2 pages).* The three-method realisation: `extract_typed`, `reify`, `query`. Map to the categorical operations: extract corresponds to the sentence-functor restricted to typed extraction; reify corresponds to the sentence-functor's right adjoint where it exists; query is the proper institution operation (satisfaction in the model). The collapse to three methods (rather than the half-dozen the literature suggests) is the engineering insight: more surface area than this is not earnings its complexity in practice.

*5. Dispatch roles (3 pages — the central novel contribution).* `OnDemand`, `AutoOnLoad`, `Decidable`. Roles are properties of the QueryClass, not the institution; a single QueryClass can wear multiple roles. `AutoOnLoad` is the one with no clean theoretical precedent: it makes the kernel's Load operation a *gated transition* — the layer cannot enter the chain unless the institution returns `Holds`. This is stronger than constraint validation and weaker than full proof obligation; it's exactly the right strength for the case where the institution can decide a sentence within its own logic but the kernel needs the answer to maintain global state coherence. Worked example: the ProofCheck institution (Verus / Lean 4 verdicts) gating the entry of `VerifiedProperty` resources.

*6. Comorphisms and the non-composition decision (3 pages — the second novel contribution).* The triadic structure $(s, m, t)$ — ExportFormat from source institution, Mini-TT term as the middle, ImportFormat into target institution. Why we don't close the registry under composition: Diaconescu's Fact 14.9 says composition of left adjoints yields only an isomorphism, not equality, and at the registry level this matters because subtle correctness bugs hide in "we composed two correct comorphisms and got a third that almost works." The engineering lesson: declared comorphisms only, no synthesised ones. Cite Diaconescu's relevant papers on heterogeneous specification and the conditions under which composition is sound.

*7. Worked institutions (3 pages).* Three concrete instantiations, with their categorical structure presented properly:
- *Lean 4 as a verification institution* — proof obligations as sentences, proof terms as model elements, the satisfaction relation as proof-checking.
- *LLM-extraction as an institution* — the awkward case: `NonDeterministic`, `Result<A,E>`-typed sentences, satisfaction defined modulo model uncertainty. Includes the design decision to treat LLM outputs as observed evidence, not derived knowledge, until a typed pipeline lifts them.
- *Julia institutions for numerical reasoning* — concrete numerical models, sentences as quantitative claims, satisfaction tied to numerical tolerance. Reuses D27 infrastructure.

*8. Discussion: theory vs. practice (1.5 pages).* What the theory predicted correctly: modular reasoning, federated knowledge, the soundness of fibred satisfaction. What surprised us: dispatch roles have no clean theoretical equivalent; the practical importance of `Undecidable` as a Verdict (the theory tends to elide this case); the awkwardness of `NonDeterministic` institutions and what they mean for satisfaction. What we had to invent that the theory didn't cover: the gating semantics of AutoOnLoad; the registry-level non-composition stance; the typed-trace-as-witness machinery.

*9. Related work (1.5 pages).* Hets and the Heterogeneous Tool Set lineage. CafeOBJ, Maude, and language-internal institution use. The Stratified Institutions work. PROV and provenance systems (briefly — the connection is that traces serve a similar epistemic role but with different formal underpinnings). Software-engineering applications of institution theory (the SE community has largely moved past institution theory; this paper argues it's time to look again).

*10. Conclusion (0.5 pages).* Institution theory works in production. The decisions that made it work were not always the ones the theory predicted. The engineering lessons — collapse to three methods, dispatch roles, registry-level non-composition — are offered as starting points for other systems builders.

**Related-work delta to make crisp.** Against Hets: Hets is a tool for working with formal specifications across institutions; Eigenius is a knowledge graph whose extension mechanism *is* institutions. Against neuro-symbolic systems: those systems usually impose a single reasoning logic; Eigenius admits arbitrarily many institutions, with comorphisms as the only inter-institution bridge. Against Datalog-with-extensions and other extensible query systems: those extend a fixed logic; Eigenius extends with new logics.

**What needs to be written before submission.** The Mini-TT-as-CIC-fragment correspondence (referenced as paper-internal but not yet formalised); a clean writeup of the AutoOnLoad gating semantics; a literature pass to make sure the "first production realisation" claim is defensible.

---

## Paper 3 — Software Engineering as a Typed Knowledge Graph: Grounding AI Coding Agents in Structural Context

**Venue.** ICSE 2027. Research track. Submission typically August 2026; conference April/May 2027. The "AI for SE" subtrack is the natural home; "Software Architecture and Design" is a viable second choice. ICSE in 2026-2027 will be saturated with agentic-coding submissions; the differentiator has to be the substrate-level argument, not "we built another agent."

**Backup venues.** FSE 2027 (later submission deadline, similar audience). PLDI if the paper leans more toward language-design contributions (the boundary-contracts formalism). OOPSLA if the contribution is framed as a typed-system contribution.

**Length.** 12 pages ICSE format (excluding references).

**Single-sentence contribution.** This paper proposes representing a software codebase — its requirements, design elements, code artifacts, tests, documentation, and contracts — as a typed knowledge graph with first-class epistemic categories, demonstrates the design through a dogfooded implementation in the Eigenius platform, and argues that this substrate structurally precludes classes of error that current text-retrieval-grounded coding agents make.

**Section outline.**

*1. Introduction (1 page).* The agentic coding moment. Cursor, Devin, Copilot, Claude Code, and the explosion of 2025-2026 — agents that operate against codebases through text retrieval. The structural information they miss: which function realises which design element, which test asserts which requirement, which module sits behind which contract, which recent commit invalidated which previously-derived guarantee. Contribution preview: a typed knowledge graph that makes these structural relationships first-class queryable data, with a working dogfooded implementation.

*2. Motivation: the structural blind spot (2 pages).* This is the empirical motivator and the section reviewers will scrutinise hardest. Two viable approaches:
- *Failure-mode analysis* over a public agent benchmark (SWE-bench Verified, or a successor). Categorise failures into (a) structurally precluded by a typed knowledge graph, (b) reducible by it, (c) not addressed. Quantitative breakdown.
- *Or* a small qualitative study: instrument an agent on N representative tasks against the Eigenius repo, log every retrieval call, identify the structural facts the agent reconstructed (correctly or incorrectly) from text. Tell the story through tracelogs.

The former is more defensible empirically; the latter is more vivid. A combined approach is strongest if effort allows.

*3. The SE knowledge graph model (2 pages).* The `urn:eigenius:se` ontology from D35: entity classes (Requirement, DesignElement, CodeArtifact, TestCase, Doc, ChangeSet, AnalysisResult, VerifiedProperty, Author, Intent), relations (realizes, satisfies, asserts, covers, depends_on, contracted_by, etc.), the bridge into boundary contracts. Emphasise the three-way intent/realisation/witness distinction — it's the structural payoff and the cleanest pitch to a software-engineering audience.

*4. The Eigenius substrate (1 page).* Brief: epistemic categories, layers, content-addressed traces, institutions. Forward-reference paper 1 for the full vision. The argument for *not* rolling our own substrate: the four-category epistemic distinction and the institution-mediated validation aren't features we'd have built ourselves for an SE-specific tool, and they turned out to matter.

*5. Validation institutions (1.5 pages).* Lint, TypeCheck, TestRunner, ProofCheck riding D14's institution mechanism. The architectural payoff: AutoOnLoad makes "the graph cannot enter a state where validated code doesn't validate" a structural invariant rather than a CI promise. Trace-based memoisation gives incremental analysis automatically. Worked example: a one-line change to a Rust function in the kernel triggers re-ingestion of just that function, re-typecheck of just the affected crate, re-run of just the impacted tests, and re-derivation of just the coverage edges that touch the changed artifact.

*6. The hybrid bidirectional / model-driven design (1 page).* The §6-of-D35 selective MDD slice: source-of-truth inverts for boundary contracts, the ESL grammar, the EigenQL surface, the gRPC API, the Eigon serialisation shape. Drift detector as an `AutoOnLoad` QueryClass on the source-tree side. The argument: MDD pays off precisely at API surfaces and protocol boundaries where source-and-spec drift is costliest, and not elsewhere.

*7. Agent integration (1.5 pages).* The read patterns (Discovery, Localisation, Coverage, Impact, History, Justification) and the write loop (Intent → source edit → re-ingest → validation → assert links → run impacted tests → promote intent to outcome). Show one or two real EigenQL queries from the dogfooded implementation. The emphasis is on what becomes structurally possible that wasn't before — agents that can ask "is this requirement test-covered?" and get a typed answer rather than a textual approximation.

*8. Implementation and evaluation (1.5 pages).* Dogfooding: Eigenius modelled in Eigenius. Per-language ingester implementation notes (`syn` for Rust, TS Compiler API for TypeScript, etc.). Layer-per-commit cadence and observed storage cost. Evaluation strategy depends on what's been measured by submission:
- *Performance:* ingestion latency per file; query latency for the §7 read patterns; index size as a function of repo size and history depth.
- *Effectiveness:* if the §2 study is structured as "with-graph vs. without-graph agent task completion," report the comparison here. Honest about confounders.
- *Case study:* concrete examples of agent tasks where the graph made the structural context immediately available, with timing comparison against a grep-based baseline.

*9. Discussion (1 page).* Limitations: per-language ingesters require non-trivial engineering; layer cadence is a tunable with operational consequences; embedding-model versioning needs a governance story; the human-authoring UX for the MDD slice is novel and untested at scale. Generalisability: the approach depends on the substrate having epistemic categories and institutions; could it transfer to a Neo4j-on-PROV system? In principle yes, with significant additional plumbing.

*10. Related work (1 page).* Knowledge graphs for code (CodeQL, Sourcegraph's universal code graph, Glean, Kythe). LSP and code-intelligence systems. AI coding agent grounding (RAG over code, retrieval-augmented Copilot variants). Boundary-specification approaches (design-by-contract, OpenAPI, JSON Schema, Pact). The delta against each: none combines typed structural representation with first-class epistemic categories and institution-based validation.

*11. Conclusion (0.5 pages).* Coding agents that ground in structural knowledge are not fundamentally harder to build than those that ground in text — they require a substrate that current platforms don't provide, but the substrate can be built. The bet is that the next generation of coding agents will be the ones that operate against typed knowledge graphs, not against grep.

**Related-work delta to make crisp.** Against CodeQL/Glean/Kythe: those systems extract structural facts from code but lack the epistemic-category layer and the bidirectional intent/realisation/witness model — they answer "what does the code do?" but not "what was the code supposed to do, who said so, and which tests witness that?" Against RAG-for-code: that approach grounds LLM context in source text; this approach grounds it in typed claims about the source. Against design-by-contract languages: those formalise the contract surface but not the realisation, witnessing, and verification chain.

**The hard problem: evaluation.** ICSE reviewers will demand an empirical contribution. The most defensible options, in order of credibility:

- *Failure-mode analysis on SWE-bench Verified or successor.* Run a representative agent (Aider, OpenHands, or whatever the 2026 leader is) on the benchmark, log failures, categorise. Then re-run with graph-grounded context provision and report deltas. This is the strongest evaluation if feasible.
- *Controlled task study with developers.* N developers complete code tasks with and without graph-grounded agents; measure completion time, correctness, satisfaction. Strong if executed well, hard to control.
- *Case study with quantitative measurements.* Document specific agent workflows on Eigenius itself, with timing and correctness measurements against a grep baseline. Weakest as a primary evaluation but viable as a supporting one.

Worth deciding the evaluation strategy by July 2026 to leave time for the empirical work before the August submission window.

**Existing assets to draw on.** D35 itself is the design substrate; the paper is largely a research-paper-shaped restatement of D35 plus the empirical evaluation plus the related-work positioning. The dogfooding implementation needs to exist in some form by submission — even partial coverage is fine if the design completeness is presented honestly.

---

## Cross-paper considerations

**Sequencing and dependencies.** Paper 1 should land first because papers 2 and 3 will reference it. If CIDR submission slips, papers 2 and 3 can still proceed by referencing the architecture directly, but they lose the "as established in [Eigenius CIDR'27]" shorthand that makes related-work positioning easier. Paper 2 and paper 3 are independent; they can be drafted in parallel.

**Author overlap.** Papers 1 and 3 will share most authors. Paper 2 benefits from a co-author with institution-theory standing — worth approaching Răzvan Diaconescu directly, or someone in the Hets community (Mossakowski, Codescu), once paper 1 is in submission and the platform's seriousness is established.

**Workload realism.** Three papers in 12-18 months is aggressive but feasible if paper 1 is leveraged from existing tex drafts and paper 3 is leveraged from D35. Paper 2 is the heaviest lift — it needs new formal writing, not just architectural restatement — and should be started early.

**What's deliberately not on this list.** The layer-aware ANN search paper (VLDB), the Mini-TT/NbE PL paper (POPL), the boundary-contracts formalism paper (FSE), and the verified-AI position piece (AIES/CACM) are all viable second-wave publications. They benefit from the first three having established the platform's existence and credibility. The drug-discovery / quantum-verification / civic-infrastructure domain papers should wait for actual domain collaborations and real results — premature publication in those venues without a domain co-author undercuts the platform's standing.

---

*Owners and target dates to be filled in once the submission strategy is committed. The next decision points: confirm the CIDR target window (September 2026 submission), identify a CALCO co-author (ideally before September 2026), and lock the ICSE evaluation strategy (by July 2026).*