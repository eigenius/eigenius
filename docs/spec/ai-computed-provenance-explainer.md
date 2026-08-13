# **Explainer: AI Computed Provenance**

Draft — 12 August 2026 · Companion to [AI Computed Provenance 1.0](./ai-computed-provenance-1.0.md) · Non-normative

## **Participants**

Editors: *to be named.* Input to the proposed **AI Computed Provenance Community Group** (proposed 11 August 2026).

**Stakeholder feedback: none yet.** The group is proposed, not chartered. This document has not been reviewed by W3C, by the group, or by implementers other than the authors of the reference implementation. Nothing below should be read as reflecting consensus.

## **Introduction**

Scientific and engineering practice is increasingly reorganized around AI systems whose reasoning cannot be inspected. Large language models and agentic tools now generate hypotheses, select methods, execute analyses, and draft conclusions across quantitative disciplines. These systems produce fluent, plausible outputs that arrive merely as assertions, unaccompanied by any checkable account of how they were reached or what assumptions they rest upon.

This exposes a structural gap. The dominant artifact of scientific communication—the prose narrative—was designed for an era where human authorship was the rate limiter and manual reading was the means of verification. Today, the constraint on generating claims has been removed, but the verification burden remains entirely on human experts who must reconstruct reasoning the narrative does not actually carry. This asymmetry produces severe failure modes: formatted citations that do not exist, statistics that do not follow from deposited data, and analytical methods that cannot be transferred across laboratories because their underlying assumptions remain tacit. Prevailing governance responses, such as disclosure requirements, detection tools, or appeals to human responsibility, attempt to manage this bottleneck through policy and labor without fixing the underlying artifact.

Current provenance records attempt to fill this gap but operate at the wrong conceptual level. They typically provide a lineage graph outlining what a claim was derived from, the model used, and the timestamp. While such a graph can document that entities are related, it cannot verify *why a claim holds*. It lacks an underlying concept of what a claim actually says or what constitutes validity. In this model, a mathematically sound derivation and a completely hallucinated one are structurally indistinguishable.

This is a fundamental representational gap. A second vulnerability compounds the issue: every field in a standard lineage record is written by the system that produced the conclusion. The relationships expressed are only as reliable as the producer's intrinsic trustworthiness.

This specification addresses both limitations by shifting the focus from flat data lineage to formal reasoning, ensuring that verification becomes a mechanically checkable property of the artifact itself rather than a manual task performed on it. A claim travels with a checkable record of what was declared, observed, and derived.

In the proposed framework, a claim has **content**: it functions as a proposition in a formal language, complete with a decidable method for determining when two claims are identical. A warrant has **structure**: it is a term that records the specific evidence it is grounded in, alongside how those groundings compose. Finally, validity becomes **decidable**: whether a warrant justifies a claim is determined by mathematical type-checking rather than manual inspection.

This framework does not attempt to formalize the underlying science. The empirical substance of disciplines like biology or chemistry remains exactly as difficult, wet, and contested as it has always been. What becomes checkable is the *reasoning about* the science—the logical inference from evidence to conclusion, the origin of each datum, the epistemic status of each claim, and the soundness of translations between domains.

This representational shift enables robust verification. By treating a warrant as a checkable object, its foundational components (leaves) can be rendered unforgeable. These take the form of statements that exist solely because an implementation performed and validated specific computational work. There is no syntax, API, or configuration parameter that allows a user or system to manually supply one. A third-party auditor can independently verify them by recomputing hashes and validating the computational work, without needing to query the original producer.

We refer to this secondary property as **Computed ≠ Asserted**. While it is the primary benefit of the specification, it relies entirely on the foundational shift in representation: a warrant cannot be made unforgeable until it exists as a formal structure.

## **The user-facing problem**

Existing provenance formats operate at the artifact level, tracking document ancestry, execution logs, and signatures. This is suitable for managing files, but inadequate for answering whether a specific conclusion is logically warranted and by what evidence. Asking this question of a standard lineage graph yields information about file ancestry rather than logical validity.

This limitation creates practical difficulties for three key stakeholders:

**The reviewer** analyzing a conclusion needs to understand its dependencies. While they can access a list of cited documents, they cannot determine which specific steps were mechanically *checked* versus logically *chosen*, or by what mechanism. An information retrieval step that found a relevant paper and a model's subjective interpretation of an ambiguous sentence appear identical within a standard provenance record.

**The second laboratory** attempting to reproduce a finding can re-run the pipeline and compare the final outputs. If the outputs differ, they cannot isolate the divergence. Because non-deterministic steps are not recorded in a form that allows independent replay, the specific point of failure remains obscured.

**The regulator** evaluating a submission must understand how much of the underlying data relies on authority. Ideally, this should be an exact metric detailing how many claims rest on assumptions rather than measurement or formal proof. Current formats cannot supply this metric because inferential steps are absent from the record, and authoritative assertions are not treated as a distinct, trackable category.

These challenges are exacerbated by the scale of automated generation. A system generating thousands of claims rapidly exceeds any capacity for manual spot-checking, necessitating structural guarantees.

## **Goals**

1. **Ensure unforgeable grounding.** A record must mathematically guarantee that any claimed computation was actually performed.  
2. **Enable independent reproducibility.** A verifier sharing no code with the producer must be able to re-check every address, certificate, and grade using only the provided record.  
3. **Make authority enumerable.** Readers must be able to extract a precise, finite list of every claim within a record that relies on external authority or assumption.  
4. **Record AI choices transparently.** Every step where the outcome was not strictly determined by the input must be documented with its authorizing agent, the considered alternatives, and a key to facilitate replay.  
5. **Account for omissions.** Records must cover every unit of input, including units that yielded no output or failed processing.  
6. **Maintain vendor neutrality.** No component of the security guarantee may depend on a specific vendor, proprietary service, or localized hardware root of trust.

## **Non-goals**

1. **Not an attribution system.** This specification does not authenticate *who* produced a record; it ensures structural integrity and reproducibility. Cryptographic signing belongs in a higher-level layer.  
2. **Not a correctness oracle.** A conforming record can faithfully document a logically flawed conclusion. The generated grades indicate *how* a claim was established, not its ultimate truth value.  
3. **Not a domain vocabulary.** The semantic meaning of domain-specific claims (e.g., in biology or law) is authored externally and is not standardized here.  
4. **Not a model evaluation tool.** This framework does not score AI models, and confidence values are explicitly excluded from being treated as logical grades.  
5. **Not a replacement for W3C PROV.** PROV effectively addresses artifact-level lineage. This specification operates a level above and is designed to map down to PROV for tooling interoperability. See [alternatives](#considered-alternatives).  
6. **Not prescriptive regarding AI architectures.** The methods used to invoke, prompt, or select models are out of scope; the specification only dictates what must be *recorded* regarding their choices.  
7. **Not a system for defeasible reasoning.** Belief revision is treated as a structural marker rather than a native implementation of non-monotonic logic.

## **The level shift: three theories, three jobs**

The core framework relies entirely on established mathematical foundations rather than novel cryptographic primitives. The specification assembles these proven concepts and defines their interfaces, allowing implementers to verify the theory against independent, established literature.

Each of these three foundational theories performs a distinct, necessary function.

### **Constructive type theory — gives claims content**

In this framework, a proposition is treated as a type; a proof is a term inhabiting it; and checking a proof is equivalent to type-checking. The constructive commitment dictates that to assert something exists, one must **exhibit it**. A proof is not merely a certificate that a witness *could* be found; it is the witness itself.

This approach yields two decidable properties that form the foundation of the system:

* **Are these the same claim?** This is resolved by the formal language's equivalence relation rather than standard string comparison. This ensures that a citation binds to a conceptual proposition rather than a specific textual representation.  
* **Does this warrant justify this claim?** This is definitively settled by type-checking.

*Absent this foundation:* claims are merely strings, sameness is strictly textual, and logical validity cannot be mechanically checked. This is the current state of flat-data provenance.

### **Justification logic — makes the warrant an object**

Artemov's Logic of Proofs treats the warrant as first-class syntax: t : A reads "*t* is a justification for *A*", providing explicit operators for composing justifications.

Practically, this means a warrant is **stored, inspected, and audited** rather than discarded immediately after being checked. An auditor can read exactly *how* a claim was justified. Multiple independent warrants for a single claim can coexist without overriding one another. Crucially, a single calculus can express a claim *justified by authority* alongside one *justified by proof*. Consequently, the epistemic grade is derived directly from the warrant's structural shape rather than attached as an arbitrary metadata label.

*Absent this foundation:* proof systems would lack an audit surface, and there would be no uniform method to represent varying types of justification (e.g., authoritative assertion vs. machine-checked proof).

### **Institution theory — lets many logics coexist**

Goguen and Burstall's institutions formalize the concept of a *logical system*—encompassing signatures, sentences, models, and satisfaction—alongside the satisfaction condition, which ensures truth remains invariant under changes in notation. **Comorphisms** act as truth-preserving translations between these logical systems.

This mechanism enables platform extensibility without requiring a universal, monolithic ontology. A statistical engine, a proof assistant, and a solver can each maintain their own syntax and verification methods. They are integrated via *declared* translations with explicitly stated semantics rather than ad-hoc middleware.

*Absent this foundation:* systems would be forced into either an impossible global logic or reliant on undocumented bridges between disparate tools.

**This formalizes the group's neutrality commitment.** True vendor neutrality is achieved structurally: the framework does not privilege any single logic, and expanding it requires registering a declared institution with specific comorphisms. This prevents vendor lock-in, as the extension mechanism relies on transparent, published translations.

## **Proposed approach**

The proposed architecture consists of seven core mechanisms. The overall integrity guarantee emerges from their combination.

**1\. Content, not files.** Every element is a resource identified by an IRI, arranged in immutable layers forming a hash-linked graph. A layer's identity is derived from a hash of its content; its positional identity incorporates its parents, ensuring that altering historical data changes all subsequent descendants.

**2\. Claims carry propositions.** Claims are not strings, but terms in a language equipped with a decidable notion of equivalence and a canonical encoding.

**3\. Validated commits emit traces.** When the implementation validates a commit that establishes a claim, it automatically writes a *trace* detailing the claim, the method of establishment, the authorizing party, and the timestamp.

**4\. Traces admit witnesses, and witnesses cannot be written manually.** A *witness* serves as the machine-checkable component of a trace. It is identified by its grade, IRI, and **a hash of the proposition**. Including the proposition hash prevents citation drift: if a resource's meaning changes, citations linked to the older hash will no longer resolve.

Witnesses are implemented as types with **no constructors**, meaning there is no syntax available to manually forge them. The author of a proof simply leaves a designated hole:

derived("urn:…:claim\_1",  HasActivity(msi, WRN),  ())  
                                                   ↑  
                            The author writes a hole here and cannot write anything else.  
                            The implementation fills it by finding a trace that admits  
                            (Derived, claim\_1, hash of that proposition), or it refuses the commit.

This ensures that grounding cannot be independently asserted by a user or system; its existence can only be mechanically verified.

**5\. Warrants compose, and only witnesses are leaves.** A specialized calculus builds composite warrants (e.g., applying a rule to a premise, offering alternatives, or specializing a general claim). Every terminal leaf in this structure is a witness. Because certificates are stored and re-checkable, verifiers can independently recreate the check rather than trusting previous execution logs.

**6\. No rule introduces an implication.** This is a subtle but highly consequential design constraint. The calculus lacks a general deduction theorem: systems cannot inherently *derive* "if A then B" merely by assuming A and concluding B. Implication can only be introduced if it is explicitly **grounded**—carried by a resource, at a specific grade, with a trace attributing it to a named party.

Consequently, every inferential leap within a record is explicitly visible as a claim backed by an identifiable authority. Systems cannot manufacture untraceable warrants. This satisfies Goal 3: users can easily filter for declared groundings to isolate exactly what a record assumes on authority.

**7\. Choices and failures are recorded.** Every non-deterministic action—whether by a model, heuristic, or human—must be logged. This includes the responsible authority, considered alternatives, rationale, and a replay key covering the *exact context presented*. Altering the context during replay registers as a documented deviation rather than a silent reuse. Furthermore, input units that fail to produce a claim generate an omission record detailing the exact reason for failure.

The record also strictly distinguishes between **vetoed** and **unvetoed** choices. If a mechanical check filters out unacceptable model outputs, the model acts only as a proposer. If all generated candidates are technically acceptable and the model selects one, its choice is final and constrained only by the audit trail. Conflating these two operational modes is misleading, so explicit disclosure of the mechanism is mandatory.

## **Key scenarios**

### **Scenario 1 — Verifiable literature citations**

A citation is a pointer to a document. It records where an author looked — not what they relied on, and not whether what they relied on holds. That gap is where a fabricated reference hides, and equally where a real reference that does not say what the argument needs hides, which is the more common and less discussed failure.

The framework splits the two things the word "citation" conflates. A **Reference** is the cited work: DOI, PMID, title, year. A **Citation** is *this* argument's use of that work — its rhetorical function drawn from a closed vocabulary (cites as authority, uses method in, obtains background from), the citing document's own label, and, when the citation is doing logical work, the **proposition being imported**.

That last field is the whole distinction. A Citation carrying a proposition is a *literature warrant*: a premise the argument composes. A Citation without one is pure provenance: it records that the author read something. Both are legitimate; conflating them is how an argument's real dependencies become invisible. In the reference encoding of a synthetic-lethality study, reference [14] carries the proposition that WRN's exonuclease and helicase activities are separable, typed *cites as authority*, and is a premise in the leg concluding that the helicase is the required activity — remove it and that leg does not close.

Once a citation carries a proposition, the proposition can be **checked**. That is what verifying a citation actually means: not that the identifier resolves, but that the claim it imports survives contact with the data behind it.

The same study, encoded as a chain with its statistical claims recomputed from the deposited public data rather than transcribed, produces three kinds of outcome — and records all three.

The headline reproduces exactly. The gene is the top differential dependency in the relevant cell lines at Q = 4.81 × 10⁻²⁴ against a published 4.8 × 10⁻²⁴, recovered by moderated-t regression over a 187 MB dependency matrix.

A competition assay also reproduces exactly — published P = 2.7 × 10⁻¹⁹, recomputed 2.74 × 10⁻¹⁹ — and the record adds what the published figure does not say: that model tests the *technical* residual, because replicates within a guide are not independent. The biological-unit model, taking the guide as the unit of analysis, gives P ≈ 2.2 × 10⁻⁶. **Neither number is wrong.** They answer different questions, and in the record they are different propositions carrying different scope of inference. The study's qualitative conclusion survives; the strength of the warrant behind it does not, and the record says which warrant supports which claim.

A third claim reproduces in effect but not in extent: a correlation's coefficient recovers exactly at ρ = −0.74 while its sample count does not — the paper reports 54, the recomputation finds 51. Classified, recorded, left visible.

This is what checking a citation produces when the citation carries a claim rather than a pointer: confirmations, refinements, and divergences, each a recorded finding rather than a note in a reviewer's margin that never reaches the record. And because no rule introduces an implication, the question *what does this argument import from outside itself?* has a finite, exhaustive answer.

One boundary is worth stating plainly. The record does not establish that a cited work exists; resolving an identifier against a registry is an external check that belongs with attribution. What the record establishes is that a citation commits its author to a specific proposition, in the open, where it can be disputed — and that a fabricated citation, while it can be declared, can only do the single job it states and cannot hide inside a derivation.

### **Scenario 2 — Reading the output of an AI scientist**

Someone is handed a body of work they did not watch being produced. The useful question is not "is this correct?" — at volume, nobody answers that by reading. The question is **where do I look?**

The record answers it by construction, through three filters. **Unvetoed choices** are the places a model selected among options the system itself could not tell apart; these are where an error enters with nothing to catch it. **Declared groundings** are what the work takes on authority. **Omission records** are what the pipeline could not process at all.

The vetoed/unvetoed split is not a gradation of confidence. It is a difference in kind, and the same pipeline exhibits both. Resolving a phrase like "these findings" to its referent is *vetoed*: a proposed antecedent that fails to type-check against the referring expression's restriction is rejected outright, and the reading dies with it — the model ranks candidates, the type system decides. Choosing among competing readings of an ambiguous sentence is *unvetoed*: every candidate type-checks, nothing mechanical discriminates, and the model's choice stands, recorded with its alternatives and its stated reasoning. A record that presents these two as the same event tells the reader nothing.

Concretely, a pipeline over one paper's 62 sentences encodes 50, leaving 1 unresolved for ambiguity and 11 for unresolved reference, with no grammatical gaps. Its 39 recorded selection decisions score 23 matching independent human adjudication, 6 wrong, 1 abstained — and 9 that the existing gold standard cannot score at all, because those readings were resolved using discourse context the gold standard was not built to evaluate.

That last figure is in the record too, and it is the point. The claim is not that the pipeline is accurate. It is that a reader who was not present can compute the pipeline's error rate from the artifact — including the part where the measurement does not yet apply.

### **Scenario 3 — Collaboration between research groups**

A second group builds on the first group's result. The first group later revises it.

Today nothing happens. The citation still formats correctly, the identifier still resolves, and the sentence in the second group's paper reads exactly as before. The conclusion is now unsupported and no mechanism anywhere says so; discovery depends on somebody noticing.

Here the citation is bound to the claim rather than to the name. A witness is keyed on the grade, the identifier, **and the hash of the proposition** — so when the first group revises what the claim says, the key changes and the second group's citation no longer resolves. The dependency breaks loudly, at the moment the dependent work is re-checked.

This is Scenario 1 seen from the other side: that one asks what an argument imports, this one asks what happens when an import changes underneath. Both are the same mechanism.

Order becomes a structural property rather than an editorial convention. In the reference encoding, the recompute layers land in two phases: the statistical analysis plans first, because their validated commit is what emits the witnesses, and the conclusions second, because they cite those witnesses and can only gate against them once they are ancestors. A conclusion cannot commit before the thing it depends on exists. The dependency ordering a paper leaves implicit is enforced by the substrate.

When two groups' contributions genuinely conflict, the resolution is itself recorded on the chain — which strategy was applied, by whom, across which layers — rather than settled in private correspondence whose outcome surfaces only as a changed number.

### **Scenario 4 — Multi-disciplinary research teams**

A screening team holds 24 IC₅₀ measurements across six compounds and four targets. Flat queries answer every question *about that data*: selectivity matrices, scaffold rollups, quality-control distributions. What they cannot answer is anything about the mechanisms underneath, and each such question belongs to a different discipline with its own formalism.

| Question | What answering it requires | Whose formalism |
| --- | --- | --- |
| What are the binding kinetics at the target, and does the steady state agree with the measured IC₅₀? | Integrating a system of ODEs derived from a chemical reaction network | Reaction-network kinetics → numerical integration |
| What inhibition constant underlies the measured IC₅₀, given the substrate concentration and the target's Michaelis constant? | A least-squares fit across dose–response readings at several concentrations | Symbolic algebra → mathematical optimization |
| Given the confidence intervals on each measurement, what range can predicted occupancy take? | Rigorous interval arithmetic with rounding-mode discipline | Symbolic algebra → validated numerics |

Each discipline keeps its own formalism. The chemist's reaction network remains a reaction network; the optimizer's problem remains an optimization problem. Nobody restates their methods in a neighbouring field's vocabulary in order to participate. Declared translations carry a claim from one system into the next, and each translation states what it preserves.

The receiving system does not take the claim's word. Committing the ODE solution fires a check that **re-integrates** the system; committing the optimization result fires a check that **re-solves** the problem. The verdict records agreement between an independent recomputation and the claim, not the claim itself.

The loop then closes numerically, which is what makes the composition worth doing. The fit recovers an inhibition constant of 17 nM. The Cheng–Prusoff relation, applied at the screening protocol's substrate concentration, predicts an IC₅₀ of 17 × (1 + 80/20) = 85 nM. The screening measurement for that compound–target pair is 85 nM with a 95% confidence interval of [72, 100]. The prediction lands inside the interval — and every input is chain-resident: the target's Michaelis constant, the protocol's substrate concentration, the fitted constant. A reader can do the arithmetic without asking anyone.

Reproducibility is carried with the verdict rather than assumed. Each one arrives with the digest of the image that computed it and the numerical metadata captured when that worker started — BLAS implementation, fused-multiply-add availability, determinism flags — so re-running the same gate on the same image reproduces the same bytes, not merely a similar answer.

This is where the framework's pluralism does real work. No universal ontology is imposed and no discipline is subordinated to another's schema. A team adds a discipline by declaring an institution and its translations, which is an extension rather than a renegotiation — and it is why the neutrality commitment is structural rather than aspirational.

The worked example uses values constructed so that every step has an independently checkable answer; real screening substitutes noisy observations, and the same chain shapes apply.

## **Considered alternatives**

The following frameworks operate primarily at the **artifact level** (tracking file lineage and signatures) rather than the **claim level** (tracking logical justification and validity). They are complementary to this specification and generally compose well with it.

### **Profile W3C PROV-O**

While profiling W3C PROV-O is a natural starting point, it presents fundamental representational limitations for this specific use case.

**PROV cannot express formal reasoning.** While RDF allows for minting new classes, the gap is structural rather than purely vocabulary-based:

| Reasoning requires | PROV's nearest construct | What is missing |
| :---- | :---- | :---- |
| The content of a claim | prov:Entity | Deliberately opaque. PROV models identity and lineage, not semantic meaning. |
| "P follows from Q by rule R" | Activity used Q, generated P, wasAssociatedWith an Agent that hadPlan R | Records that an event occurred. R is an opaque prov:Plan; nothing states its logic or checks conformance. |
| A checkable warrant | — | PROV has no proof objects or logical validation mechanisms. |
| Composing warrants | Activity chaining | Chains *activities*, not *logical justifications*. Cannot validate a composite warrant. |
| Truth-preserving vs. guessed | — | wasDerivedFrom is highly general and covers both equally. |
| Validity | PROV-CONSTRAINTS | Constrains structural well-formedness (ordering, uniqueness), not logical validity. |

To express reasoning, a system requires content with a decidable identity and a strict checking relation—features provided by the [three foundational theories](#the-level-shift-three-theories-three-jobs) discussed earlier. Furthermore, PROV's entity/generation model struggles with multiple independent justifications for a single proposition, often resulting in duplicated entities rather than unified claims.

Most importantly, PROV graphs are entirely producer-writable. A system can assert a wasDerivedFrom relationship without ever executing the derivation, and the PROV statement remains structurally valid.

**Future integration:** The group intends to publish a downward mapping to ensure records remain consumable by existing PROV tooling. This mapping will be inherently lossy (flattening certificates into standard wasDerivedFrom edges), but it will provide essential backwards compatibility.

> **Prior art to verify:** The Proof Markup Language (PML) from the Inference Web project (McGuinness, Pinheiro da Silva, mid-2000s) represents the closest historical precedent for reasoning-level provenance. PML's justification layer modeled inference steps with antecedents and rules. Understanding why the W3C Provenance Incubator Group narrowed its scope away from PML-style justification toward artifact lineage is critical context for this ongoing work.

> **Unverified — check before circulating.**

### **Signed attestations (in-toto / SLSA)**

SLSA provides a highly relevant model centered on build-platform attestations rather than publisher assertions.

However, SLSA derives its security by **trusting the builder**—the guarantee relies on a cryptographic signature from an approved platform. The framework proposed here derives non-forgeability from the **record's structural properties**: witness types lack constructors, a fact a verifier can independently confirm. The two approaches are complementary; signing handles identity and attribution, while this framework ensures computational validity.

### **Verifiable Credentials**

Verifiable Credentials (VCs) offer issuer-signed claims with mature revocation systems. Like SLSA, they establish authenticity rather than computational proof. VCs are a strong candidate for integration at the attribution layer.

### **Log every model call**

Storing all prompts and responses creates massive, unwieldy datasets that do not structurally compose. A log of prior API calls does not inherently explain the logical dependencies of an isolated claim. Furthermore, raw logs pose severe data privacy risks. Recording explicit decisions, alternatives, and replay keys is vastly more efficient and less disclosive.

### **Trusted execution / remote attestation**

Hardware attestation proves that a specific binary executed within a secure enclave. It does not evaluate the logical soundness of the binary's output, and it requires trusting a specific hardware vendor—violating the group's strict vendor neutrality requirements.

### **Require formal proof everywhere**

Restricting the system exclusively to machine-checked claims is impractical and would render the framework unusable for most real-world research. This is why the system supports four distinct epistemic grades. The record remains highly valuable even before formal proofs are applied, as it clearly demarcates the boundary between checked computations and authoritative assumptions.

### **Confidence scores plus human review**

This remains the current de-facto industry practice. However, confidence scores do not compose across inferential steps, lack standardization between distinct models, and provide no dependency tracking. Furthermore, manual review cannot scale alongside automated generation. The specification allows for confidence metadata but strictly prohibits substituting confidence scores for structural epistemic grades.

## **Security and privacy considerations**

Summarized from [§11 of the specification](./ai-computed-provenance-1.0.md#11-security-and-privacy-considerations).

**Addressed vulnerabilities:** Silent substitution of propositions beneath stable citations; undetected historical alterations; undocumented epistemic grades; asserted-but-unperformed computations; and the artificial inflation of coverage metrics by silently dropping input.

**Unaddressed vulnerabilities:** A malicious producer claiming conformance while generating non-conforming, structurally inconsistent records will fail local verification, but this requires an active auditor. The absence of a strict attribution layer leaves this open to spoofing. The framework also does not address underlying implementation defects, accurate documentation of factually incorrect conclusions, or general system availability.

**Privacy constraints:** Conforming records carry detailed source spans, extracted text, model rationales, and rejected alternatives. For clinical, proprietary, or embargoed data, this level of disclosure may be unacceptable. The specification permits redaction, provided it is explicitly declared and addresses are correctly recomputed over the redacted content to prevent cryptographic mismatches.

**Cryptographic exposure:** Content addressing inherently reveals content equality. If two distinct parties hold the identical dataset, they will generate identical addresses. While this allows laboratories to verify shared data without direct disclosure, it also permits observers to confirm hypotheses about undisclosed data by matching hashes.

## **Open questions**

1. **Cross-binding agreement.** The specification outlines four necessary properties for a proposition language. Whether these are sufficient to guarantee that two independent implementations interpret a proposition's *meaning* identically remains unsettled, though they are currently sufficient for verification within a single binding.  
2. **The PROV mapping.** The structural mapping to W3C PROV-O must be formally drafted.  
3. **Attribution integration.** The specification mandates that signing layers bind to content addresses rather than serializations. The group must determine if this constraint is sufficient and which existing attribution model (e.g., VCs, SLSA) should be officially recommended.  
4. **Registrations.** The reference binding utilizes an unregistered media type and an unassigned CBOR tag. Both require formal IANA registration prior to standardization.  
5. **Identifiers.** The reference implementation relies on vendor-namespaced IRIs. The working group must decide whether to mint a dedicated namespace, adopt the existing ones, or define a new registry model.  
6. **Grade propagation constraints.** The specification dictates that the Verified grade survives composition only when every sub-warrant (including the inference rule itself) is also Verified. This strict propagation rule is not yet active in the reference implementation, which currently projects grades from the landing warrant.  
7. **Selective disclosure proofs.** Proving that a redacted record is a mathematically faithful subset of a specific original—rather than merely an internally consistent standalone record—requires further specification, likely utilizing hash-tree constructions.  
8. **Deliverable scope.** The proposed charter references "architectural guidelines ensuring the protocol remains inspectable and protected from commercial capture." Because anti-capture mechanisms within W3C are primarily procedural rather than technical, this likely represents two distinct deliverables that should be separated for project management clarity.

## **References**

**The specification.**

* [AI Computed Provenance 1.0](./ai-computed-provenance-1.0.md) — the specification this document explains.

**Foundations.**

* Martin-Löf, P. (1984). *Intuitionistic Type Theory.* Bibliopolis. And Coquand, T. and Huet, G.  
  (1988), "The Calculus of Constructions", *Information and Computation* 76(2–3). The  
  propositions-as-types substrate; the binding's type theory is a fragment of the Calculus of  
  Inductive Constructions, the same family underlying Rocq and Lean.  
* Artemov, S. (2008). "The Logic of Justification", *Review of Symbolic Logic* 1(4). And Artemov, S.  
  and Fitting, M. (2020), *Justification Logic: Reasoning with Reasons*, Cambridge University Press.  
  The warrant calculus is a fragment of the Logic of Proofs.  
* Goguen, J. and Burstall, R. (1992). "Institutions: Abstract Model Theory for Specification and  
  Programming", *Journal of the ACM* 39(1). Signatures, sentences, models, satisfaction, and the  
  satisfaction condition; comorphisms as truth-preserving translation between logical systems.

**Prior art in the alternatives section.**

* W3C PROV (PROV-DM, PROV-O, PROV-CONSTRAINTS); Verifiable Credentials Data Model; C2PA; in-toto and  
  SLSA; RO-Crate.  
* McGuinness, D. and Pinheiro da Silva, P. — the Proof Markup Language and the Inference Web project.  
  The closest precedent for reasoning-level provenance, and the one whose relationship to the W3C  
  provenance work most needs establishing.

> **Unverified.** Every characterisation in the alternatives section — W3C PROV, the Verifiable

> Credentials Data Model, C2PA, in-toto/SLSA, RO-Crate, and the PML history — is written from the

> editors' understanding and has **not** been checked against primary sources or against those

> specifications' current state. Citation details above are approximate where publication data was not

> confirmed. All of it must be verified before this document is circulated.
