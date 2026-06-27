# D62 — The encoding engine: prose → typed trees (the generation front-end)

*Status: **proposed** (June 2026) · design specification. The **generation** counterpart to
[D61](d61-llm-based-encoding-methodology.md): D61 defines the typed *target* (the decision
layer, §5), the *check* (the gates + faithfulness oracle, §6–7), and the *methodology* (the
descent, §3); **D62 designs the engine that produces candidate typed trees into that target.**
Downstream of D61's Phase-1 foundation and **research-grade** — never shipped without D61's
check. §4 (language resources) is a **candidate landscape to be grounded** (verified research),
not vetted fact.*

*Companion documents: [D61](d61-llm-based-encoding-methodology.md) (target + check — the engine
emits into it and is guarded by it), [D8 CompleteJson](d8-complete-json-component.md) (the
existing LLM-completion component the first slice extends), [D32 FormulaTerm](d32-chain-mirrored-mini-tt-inductives.md)
+ [D47 EigenTT fragment](d47-chain-mirrored-eigentt-type-fragment.md) (the term substrate the
engine targets), [D18 ontology-as-types](d18-ontology-as-types-resolution.md) (Luo / MTT — the
nouns-as-types half), [D43 retrieval](d43-text-and-vector-retrieval.md) (sense grounding +
retrieve-first), the `grounding` skill (how §4 must be authored).*

*The deterministic **DCG generation engine** — the categorial type system, the parser
(combinators + chart), and the English lexicon (content + function-word track) — is specified
end-to-end in **[D63](d63-dcg-engine-english-grammar.md)**, which lifts the realized core from
here (§3, §8.6–8.8.1). D62 is the **encoding architecture that consumes it**: the LLM augmentation
path (§8.7.8), the faithfulness boundary (§5, D61), and the encoding institution (§6, §8.8.2–8.8.5).*

---

## 1. Motivation & relation to D61

D61 establishes *what* a faithful encoding is and *how it is checked*, and shows (D57) that
encoding done by hand is slow and leaves reconstruction debt. The missing piece is the
**on-ramp**: an engine that turns prose into the typed, composable, witnessable building blocks
the agent reasons with — at scale, not by hand.

The relationship is a strict, one-directional dependency:

```
prose ─►[ D62 ENGINE ]─► candidate typed trees ─►[ D61 check ]─► admitted building blocks
                              (emits into D61's target)   (oracle #1/#2 + human boundary)
```

Consequences that govern the whole design:
- **The engine is the untrusted step.** "prose → typed trees" *is* the
  autoformalization / semantic-parsing problem, which D61 §10 documents as the bottleneck whose
  output is systematically over-trusted (Herald 97 %→66 %). So the engine is **inseparable from
  D61's check**; its output enters **provisional** (Declared/candidate) and climbs grade only on
  check + human sign-off. An engine without the check produces *prose with false precision* —
  worse than prose.
- **The engine is built last.** It needs (a) D61's typed vocabulary to emit and (b) D61's check
  to validate. Both are the Phase-1 work. Building the engine first is a generator emitting into
  a void.
- **Scope it, don't boil the ocean.** Schema-constrained, domain-bounded extraction
  (SPIRES-style), not open-domain autoformalization.

## 2. The pipeline

Five stages; each names its input, output, and **failure mode** (where it fails closed):

| Stage | In → Out | Mechanism | Failure mode |
|---|---|---|---|
| **1. Lexicon** | prose span → {category, sense} per word/span | **LLM proposes** a categorial type + a candidate sense (untrusted) | wrong category/sense → caught downstream by composition / faithfulness, not here |
| **2. Composition** | categorised spans → a typed term | **type-logical / categorial** derivation (Carpenter, §3): the derivation *is* the term (Curry–Howard) | a span that doesn't compose **doesn't type-check** → fail-closed (the first real check) |
| **3. Grounding** | term heads → shared IRIs | retrieve-first (D43) + adopt OBO / schema.org / the D61 `verify:` vocabulary | an ungroundable head → a discovery target (D61 §3) or a new-term proposal |
| **4. Target mapping** | typed lambda / HOL term → EigenTT Props + witness scaffolding | translate the model-theoretic term into constructive EigenTT (§3, the real engineering) | an untranslatable construct → recorded Limitation (D61 §5) |
| **5. Check** | candidate tree → graded, admitted (or rejected) | **D61** oracle #1 (compose/type-check) + oracle #2 (faithfulness) + human at the witness boundary | unfaithful / ungrounded → fail-closed finding; tree stays Declared until confirmed |

The output of stage 5 is a **checked, graded building block** that accumulates (retrieve-first
finds it next time) — the compounding that makes the agent's reasoning improve over time.

## 3. The formal spine

The engine's core is a principled, checkable composition into a typed substrate — not an opaque
LLM extraction. Three type-theoretic-semantics traditions contribute; one of them is the
substrate Eigenius already is.

- **TTR — Cooper's *Type Theory with Records* (the substrate match; records-first).** TTR builds
  natural-language meaning on **record types** (labelled fields of *types*) whose witnesses are
  **records** (labelled fields of *objects*), a record being of a record type iff its fields
  match — and a **proposition is a type, true iff it has a witness.** That is **exactly the
  Class-as-record-signature** Eigenius uses: a `Class` is a record type (a dependent record /
  Σ-type) with `requires` = mandatory fields and `recommends` = optional fields; a `Resource` is
  a record = a witness; `resource : Class` + the reasoning witness model is TTR's `a : T`. TTR is
  **intensional, uses types not possible worlds**, and makes **types first-class objects to
  reflect on** — which Cooper ties to *reflection* in programming languages, i.e. Eigenius's
  `reflection:` layer. It is a **mature semantics built natively on record types** — the
  reference to mine for record-typed semantics done rigorously (dependent fields; manifest /
  singleton fields; record subtyping; the type `Type` and stratification — Cooper, Appendix
  A6–A11).
- **Carpenter's type-logical semantics (the composition mechanism).** Categorial slots +
  Curry–Howard derivation-as-term — *how words compose* into that record structure (pipeline
  stage 2). TTR gives the target; Carpenter gives the compositional route to it.
- **MTT-semantics — Chatzikyriakidis & Luo (nouns as types; *both* model- and proof-theoretic).**
  Common nouns as types with coercive subtyping (the lexical-entry shape, D18) — and the framework
  Luo et al. argue is **both model-theoretic and proof-theoretic**, with NL semantics **verified in
  Coq**. That dual nature is a second, framework-level answer to the model-vs-proof worry (TTR
  answers it via records); the Coq verification is the precedent for the engine's check / the Lean
  correspondence (D28/D30); and its impredicative `Prop` ≈ Eigenius's Prop universe (D46).
- **EigenTT** (D32/D47) is the term substrate all three target. Eigenius's impredicative `Prop`
  universe (D46) is the same construction as the MTTs' (UTT / pCIC), so EigenTT *is* a member of
  this family.

These are not three rival imports but **one stack**, and Luo's **dependent categorial grammars**
(DCGs; Chatzikyriakidis & Luo, Ch. 7.3) are the glue: *dependent Lambek categories are MTT
semantic types*. So Carpenter says **how to compose**, MTT/DCG says the **categories are the
dependent types**, and TTR says those **types are records** (= Eigenius Classes). The pipeline's
stage 2→4 (compose → ground → target) is a single type-theoretic move, not a translation between
three formalisms. And **DTS** — Bekki's Dependent Type Semantics (§4.1) — is the *working
instance* of this family: a CCG→Σ-type parser (`lightblue`) with a native type-check + the Wani
prover, the closest existing system to the engine.

**The model-vs-proof gap largely closes.** D62's earlier worry — translating Montague HOL over
*models* into Eigenius's constructive, *proof-carrying* EigenTT — was an artefact of a
possible-worlds target. **With TTR as the semantic target the gap shrinks to near-identity at the
record level:** TTR is already record-based, intensional, witness-true, and Martin-Löf-rooted —
the same family as EigenTT (D32/D47). The remaining engineering is the precise **EigenTT ↔ TTR
record-type correspondence** (Cooper, Appendix A10–A11), not a model→proof rewrite. This is the
strongest formal news in the design: *the engine targets a substrate Eigenius already is.*

**Grounding status (MTT free appendices, primary-read).** The substrate match is grounded, not
just asserted:
- **Records = Σ-types** — App 7 states it outright (*"Coq's record types are Sigma-types"*) with
  worked class-signatures (`Record … {h:> Human; I: Irish h}` — fields = `requires`, coercion
  field, proof field); App 2 gives the Σ-rules. An Eigenius `Class` *is* a Σ-typed record.
- **Impredicative `Prop` = D46** (App 3: impredicative + the ∀-encoded connectives; D46:
  impredicative + proof-irrelevant).
- **`Fin(n)` finite types = closed enumerations** (≈ `allows_only`); **Π = functions** — the
  EigenTT↔MTT constructor map (App 2).
- **Signatures** (App 5, LFΔ) ≈ Eigenius layers / class-signatures; constructors are specified by
  *declaring constants* ≈ EigenTT's declared fragment.
- **NL inference verified by Coq proof** (App 7: valid entailments `Qed`, invalid ones `Abort`) —
  the concrete precedent for oracle #2 / the Lean correspondence (§6–7).
- *Correction:* coercive subtyping is shown in App 7 (Coq `Coercion`, `Surgeon >-> Human` ≈
  `subclass_of`) and the paywalled Ch. 2 — **not** App 5 (LFΔ is signatures).

## 4. Language resources — verified catalog

Grounded by a verified `deep-research` pass (adversarial 3-vote verification; 30 sources; run
`wea30vq2b`). License + maintenance facts are the most volatile — **re-verify before adoption.**
What *survived* verification is below; resources fetched but whose claims didn't clear the
verification budget are flagged **pending** (absence ≠ irrelevance).

### 4.1 The closest existing system — DTS / `lightblue` (a near-reference implementation)

**Daisuke Bekki's `lightblue`** is the single closest artifact to this engine: a **CCG parser
whose derivation maps homomorphically (Curry–Howard) to Dependent Type Semantics (DTS)** — a
Martin-Löf dependent type theory over **Σ-types with proof-carrying witnesses** — covering
**stage 2 (composition), stage 4 (target = Σ-types), and stage 5 (check)** natively. Lexical
entries are triples *(form, CCG category, DTS preterm)*, so the derivation literally carries the
typed term and ill-formed entries surface as type-check failures (the "Semantic Felicity
Condition"); a bundled DTT theorem prover, **Wani**, resolves underspecified types (anaphora /
presupposition as proof search). **License: BSD-3-Clause** (verified against the LICENSE file;
caveat: `package.yaml` carries a stray `AllRightsReserved` Cabal default — the LICENSE file is the
operative grant). **Actively maintained** (commits into 2026). DTS is a **fourth records-first /
Σ-type semantics** alongside TTR / MTT / Carpenter (§3) — and the only verified one shipping a
*parser + native type-check*. **Critical risk: strongest for Japanese; English is a thinner, NLTK-fronted path** — the README
requires Python **NLTK + NLTK data** as the English morphological analyzers (English has *no local
options*, vs. three configurable Japanese analyzers — KWJA / JUMAN / JUMAN++), and the repo bills
itself as *"A CCG parser for **Japanese** with DTS-representations."* So lightblue-for-English is a
**Haskell-DTS-core + Python-NLTK-front bridge**, with English a thinner adapter over the
Japanese-native core — the single largest caveat for an English-first platform. *(Verified against
the repo README, June 2026.)*

### 4.2 Type-theoretic composition / target (stages 2 & 4) — all research-grade

- **`lightblue` / DTS** (§4.1) — BSD-3; dependent-type-native; parser + check. *The deeper-path spine.*
- **Grammatical Framework (GF)** — a Martin-Löf-based **type-theoretical grammar** with dependent
  types in abstract syntax and abstract-syntax-as-interlingua; the multilingual composition/target
  backbone. **Import-friendly licensing:** the libraries (Resource Grammar Library, runtime) are
  LGPL/BSD with an explicit carve-out that application grammars may be relicensed freely — only the
  *compiler* is GPL. Caveat: GF's dependent types are "not very useful for most NL grammars" in
  practice; bundled lexicon data may carry separate licenses.
- **`ccg2lambda`** — Apache-2.0, Python; CCG derivations → typed-lambda via YAML templates, Coq as
  the entailment guard. **Negative result:** targets *simply-typed HOL*, **not** dependent
  types/Σ-types — a generic typed-logical target, not the Martin-Löf specialization. (Avoid its
  historical C&C parser dependency — non-commercial; prefer depccg / Jigg.)
- **Chatzikyriakidis & Luo, "Natural Language Inference in Coq"** (JoLLI 2014) — MTT-semantics in
  Luo's impredicative **UTT + coercive subtyping** (genuine Σ-types; the D18 / §3 anchor); proves
  72/77 FraCaS. **But composition is *manual*, not parsed** (a GF front-end was future work) — the
  proof-of-concept that Σ-type NLI is provable, not yet auto-parsed.

> All four type-theoretic systems are **research-grade, not production-hardened.**

### 4.3 Grounding resources (stages 1 & 3) — license verdicts

| Resource | Role | License | Verdict |
|---|---|---|---|
| **WordNet** | senses (1) + head grounding (3) | **OSI-approved** (Jul 2025, SPDX `WordNet`); commercial OK, attribution only | **adopt** ✓ |
| **VerbNet** | predicate-argument valence (1/3); ships FrameNet/PropBank maps (SemLink) | permissive HPND/X11 (U. Colorado), commercial use explicit | **adopt** ✓ |
| **PropBank** | predicate-argument valence + SemLink hub | **CC BY-SA 4.0 (copyleft / ShareAlike — viral)** | reference only, **not** a hard dependency ⚠️ |
| **BabelNet** | multilingual senses / links | custom **Non-Commercial** (research institutions only) | **avoid** ✗ (paid Babelscape path is separate) |
| OBO / schema.org | domain grounding | already in Eigenius | adopt (in-repo) |

SRL caveat: **AllenNLP is dead** (archived Dec 2022) — don't build the role layer on it.

### 4.4 Parsing backbone (stages 1–2)

Earley / **LRE(k)** (§ above) is the verified chart-parsing skeleton (ambiguity-tolerant,
incremental, semantic-action-friendly), lifted to typed composition via **parsing-as-deduction**.
**Pending verification** (fetched, didn't clear the budget): depccg, EasyCCG, neural supertaggers,
spaCy / Stanza / UDPipe, Link Grammar, the GLR/Tomita/Leo comparison.

### 4.5 Stage-1 proposer + MR front-end — verified

Grounded by a second `deep-research` pass (run `wh196l0zz`; 21 sources, 3-vote verification).

**Stage-1 constrained-LLM proposer** (grammar-constrained decoding — emits structurally-valid
candidate terms, not free text):
- **XGrammar** (Apache-2.0 ✓) — strongest; constrained decoding over **JSON-Schema / regex /
  general CFG**; maintained (v0.2.2, 2026-06); the default structured-gen backend for
  vLLM/SGLang/TensorRT-LLM/MLC. *Caveat: the README's "100%" is overreach — grammar-compile
  failures, a ~2.21% invalid-JSON eval, a vLLM bypass bug; a **structural/format** guarantee, not
  semantic.*
- **llguidance** (MIT ✓) — co-equal; **arbitrary CFG (Lark variant) + JSON-Schema subset + regex**
  via token masks; **fails closed** (errors, never silently-invalid) on unsupported schemas — ideal
  for feeding a type-checker; maintained (v1.0.0 2025-06 → v1.7.6 2026-06; Microsoft → guidance-ai).
- **Outlines** (Apache-2.0 ✓) — viable *fallback*; its CFG path is **experimental** and weak on
  pathological recursive JSON-schemas → not the sole CFG engine.
- **OntoGPT/SPIRES** (BSD-3 ✓) — a LinkML-schema proposer, but **post-hoc conformance, NOT
  constrained decoding** (a validated-against contract, not a decode-time guarantee) — softer;
  *maintenance uncertain (latest tagged v1.1.1, Apr 2024 — re-verify).*
- **Theory anchor:** grammar-constrained decoding *can* guarantee CFG-membership by token-masking
  (Park, Zhou & D'Antoni, ICML 2025, arXiv:2502.05111) — a **structural/format** guarantee only,
  conditional on sound tokenizer↔grammar alignment, and it can distort the model's distribution.
  **The dependent-type checker remains the semantic authority.**

**Parser / meaning-representation front-end** (deeper path):
- **DELPH-IN ERG** (MIT ✓) — the cleanest *maintained* bridge: broad-coverage HPSG mapping English
  → **Minimal Recursion Semantics** logical forms; ERG 2025 (2025-05), commits into 2026. (MRS is
  *underspecified* LF; the MIT covers the grammar repo — the LKB/ACE/PET processing tools have
  their own licenses.)
- **depccg + ccg2lambda** (MIT *code* ✓) — CCG → lambda/HOL logical forms; capability-rich but
  **unmaintained since 2023**, and its English models are **CCGbank/LDC-encumbered** (code clean,
  weights not).
- **IBM transition-amr-parser** (Apache-2.0 *code* ✓) — text → AMR graphs (Penman), SoTA Smatch;
  but **stale (2023)** and **training needs proprietary LDC AMR corpora** (LDC2017T10; inference
  checkpoints are free).
- **AVOID C&C** ✗ — non-commercial academic license + dead since 2019.

**Code-vs-data license split (important):** several tools are permissive in *code* but their
English/training *data* is LDC-encumbered (CCGbank LDC2005T13; AMR-2.0 LDC2017T10) — not
redistributable. The CODE imports cleanly; reproduction/training does not.

**Coverage gap (un-verified-in-this-pass, not negative):** instructor, Microsoft Guidance proper,
LMQL, llama.cpp GBNF, EasyCCG, recent neural supertaggers, Boxer / PMB, UCCA, spaCy, Stanza,
UDPipe, Link Grammar — no surviving verified claims; revisit if needed.

### 4.6 Recommendation (minimal viable stack)

- **Pragmatic first slice:** an **LLM grammar-constrained proposer** — **XGrammar** (Apache-2.0) or
  **llguidance** (MIT), emitting structurally-valid candidate terms (CFG/JSON-Schema) — → a **typed
  contract** → the **type/proof checker as the guard** (the verified CHECK-stage pattern shared by
  `lightblue` DTS+Wani, `ccg2lambda` Coq, MTT-in-Coq; D8 `CompleteJson` is the in-repo substrate).
  Grounding: **WordNet + VerbNet** (permissive); PropBank reference-only; **avoid BabelNet**.
- **Deeper categorial / dependent-type path:** **`lightblue` / DTS** as the dependent-type-native,
  license-clean spine (the only verified Σ-type + Curry–Howard + native-check system); **GF** as
  the multilingual composition backbone; the **DELPH-IN ERG** (MIT, maintained) as the English→MRS
  parser front-end where one is needed; Chatz&Luo's UTT/Coq as the provable-but-not-yet-parsed
  proof-of-concept. **Open risk: lightblue's English path is NLTK-fronted / Japanese-secondary**
  (§4.1, §4.7 5a) — betting on it for English buys a Haskell+Python bridge, not a single-language front-end.

### 4.7 Open questions (still open after both passes)

The two verification passes *answered* "which constrained-decoder" (XGrammar / llguidance, §4.5)
and "which CCG/MR tools are permissive & maintained" (ERG ✓; depccg/IBM-AMR stale; C&C avoid).
These remain genuinely open:

- **(5a) — partly resolved (June 2026).** `lightblue`'s English path is **NLTK-fronted** (Python
  morphology → Haskell DTS), with *no local options* vs. Japanese's three analyzers; the repo is a
  *Japanese* CCG parser, so English is a thinner adapter over the Japanese-native core (verified
  against the README). **Implication:** as an English MR front-end it is itself polyglot
  (Haskell + Python), so it does *not* buy a single-language English path. *Residual:* whether an
  English DTS corpus/lexicon exists to evaluate coverage against — still no verified claim.
- **(5b)** Is **Wani** (the DTT prover) a separately-distributed, separately-licensed artifact, or
  only bundled inside `lightblue`? *Unresolved — lead: a 2025 paper (ACL BRIGAP, `2025.brigap-1.1`)
  presents `lightblue` + **Wani** as paired-but-named components, suggesting Wani is at least
  conceptually separable; distribution & license still unverified.*
- **(5c)** Has a **GF → dependent-type (MTT/UTT or DTS)** pipeline materialized (Chatz&Luo's "future
  work")? *Unresolved — but a candidate lead surfaced: **GLIF / glifkernel** (GF + the MMT logical
  framework) — to investigate.*
- **OntoGPT/SPIRES maintenance** — conflicting signals (tagged v1.1.1 Apr 2024 vs a footer "Apr
  2026" judged a misread); verify the real latest release before relying on it.
- **Which term-language grammar** (S-expr / typed-lambda / contract DSL) for the EigenTT target is
  both an *unambiguous CFG* (for Lark/LALR-style engines) **and** robustly compilable by
  XGrammar/llguidance without their grammar-compile/termination failure modes? *(The concrete next
  design question for the stage-1 slice.)*

## 5. The LLM's role and the faithfulness boundary

The division of labour is the whole point:
- **LLM = lexicon / sense proposer** (stage 1) — *untrusted*; it proposes categories and senses
  for novel/technical prose where no lexicon exists.
- **Type logic = compositional check** (stage 2) — a wrong composition fails to type-check.
- **D61's faithfulness oracle + the human boundary = the semantic check** (stage 5) — because a
  *well-formed* term can still mis-capture intent if the LLM's lexicon choice was wrong (the
  faithfulness gap does not vanish; it moves to the lexicon).

So every tree the engine emits is **Declared / candidate** until D61's check + a human sign-off
climb its grade. This is the structural reason D62 cannot be a standalone oracle.

## 6. The engine as an institution

The engine's natural home in Eigenius is a **dispatched institution** — the same pattern the
Julia (D27), Lean (D28), R (D55/D56), and statistics (D52) computations already use, and the
reasoning checker itself. Realizing it this way is not cosmetic; it gives the engine three
things for free:

- **A first-class dispatch + execution path.** Given prose, the engine is invoked through the
  kernel and runs on the **runtime substrate** (D26/D56/D60 — native or the `oci` runtime, as R
  and the schema.org generator already do), authored via the **external-institution lifecycle**
  (D31).
- **The correct epistemic status by construction.** An institution dispatch emits a
  `DerivedResource` under a `ProgramTrace → IsDerivedAs` (D56) — so the engine's output is
  **Derived** ("the kernel attests the engine computed this"), *never* Verified. That is exactly
  D62's provisional-until-checked discipline (§5), **enforced by the framework** rather than
  bolted on.
- **Three roles, cleanly separated.** *Generation* (prose → tree) is **on-demand** (D31): you
  invoke it to encode a piece of prose, realized as a commit-capable `FIBER … INTO` query (which the
  kernel *requires* to carry the OnDemand role, §8.8.2). The engine *generates*; it does not gate
  arbitrary commits. *Felicity* (does the committed tree type-check?) is an **AutoOnLoad** role on
  the emitted `lexicon:` resource classes — structural, deterministic, fail-closed (§8.8.2); it is
  the §8.6 commit-time check realized as a D14 gate, **not** the engine admitting data.
  *Faithfulness* (does the tree mean the prose?) is the separate **D61 verification institution**
  (LLM-judge + human, Verified/Fails). So the split is a clean **generation institution (D62,
  Derived) + verification institution (D61)** pair — two institutions in one framework, mirroring
  how the reasoning institution verifies what other producers emit. (§8.8 gives the full mechanism.)

**The deeper reading (and the correct direction).** Institution-theoretically, autoformalization
is a **translation between logics** — a *comorphism* from the informal / natural-language source
into the EigenTT / reasoning institution (D10 Grothendieck institution protocol; cf. the typed
merge comorphisms of D37). Carpenter's type-logical semantics (§3) is itself a syntax→semantics
translation of this shape. In this language the **faithfulness gap is precisely that the
comorphism's satisfaction-preservation is not guaranteed by construction** — the LLM lexicon
makes the translation *approximate* — which is exactly why a verification institution + the human
boundary (§5) are mandatory, not optional. (Stated as the intended direction; pinning the actual
institution-theoretic obligations is itself a D62 design item, not claimed as settled.)

## 7. Build staging

The DCG **generation engine** — the categorial core, the parser, the lexicon — is staged in
**[D63 §8](d63-dcg-engine-english-grammar.md)** (the Slice 0–6 plan; it is now the home for what this
section used to sketch as "the type-logical composition core" and "broader parsing front-ends").
D62's own staging is the **autoformalization layer** over that engine, and the discipline is
unchanged: **each slice ships *with* D61's check, never before it** — the engine emits *candidate*
trees (Derived), and D61's faithfulness oracle + the human boundary climb their grade (§5). The
D62-specific slices are the **LLM augmentation path** (§8.7.8) feeding that same check, and the
**encoding-institution** wiring (§6, §8.8.2–8.8.5).

## 8. Bootstrapping the lexicon

The lexicon (stage 1) is the engine's **bottleneck** — and the only genuinely new linguistic
resource (the composition rules are a small universal set; §3). It is bootstrapped from existing
permissive resources, validated formally, and codified at a graded witness — never hand-built, never
trusted unchecked. **The primary build is two parts:** the **deterministic structural import** of
WordNet — the general English *content* framework (**§8.7**, built) — and the **hand-authored
function-word track** (the closed class that carries the compositional weight;
[D63 §4–5](d63-dcg-engine-english-grammar.md)). The **LLM-proposer loop (§8.1–8.5) is now the
*augmentation* path** (domain vocabulary + scale, §8.7.8), *not* the primary lexicon build. The
structural bulk — synset→type, hypernym→subclass,
frame→category — is *mechanical*; the LLM is reserved for where it is actually needed. Because
compositionality is lexicalized in the type system (§8.4), validating the lexicon largely validates
composition too.

### 8.1 Inputs — existing data, per entry-field + license

> **Scope (since Path B).** §8.1–8.5 describe the **LLM-augmentation proposer loop** — the path for
> *domain vocabulary and scale* (§8.7.8), **not** the primary lexicon build. The general content
> lexicon is the deterministic WordNet import (§8.7, built); the function-word track is hand-authored
> ([D63 §4–5](d63-dcg-engine-english-grammar.md)). The loop's propose → gate → battery → grade
> *method* (§8.3) remains the augmentation/scale method, under the same kernel felicity gate + FraCaS
> discipline — retained here as that method, not as the primary path.

A lexical entry is `(form, sense, category, meaning-term, grounding)`. Each field is seeded from
verified existing data (§4):

| Source | Provides | Seeds | License |
|---|---|---|---|
| **WordNet** | synsets: lemma, POS, gloss, **hypernym taxonomy**, sense keys | the *content-word* work-list; the **sense**; **grounding/type** (hypernym→IRI, esp. nouns); meaning seed (gloss) | OSI-approved ✓ |
| **VerbNet** | verb classes: **syntactic frames**, thematic roles, **selectional restrictions**, **semantic predicates** | verb **category** (frame→CCG type); argument **types** (restrictions→dependent fields); **meaning-term** skeleton | permissive ✓ |
| **Eigenius ontologies** (schema.org/OBO/`verify:`) | existing typed IRIs | **grounding** targets — retrieve-first; synonyms share a ground | in-repo ✓ |
| **FraCaS** (+ JSeM) | ~346 **gold labelled inference problems** by phenomenon | a **non-embedded eval reference** for function-words / constructs | **GPL-3.0** (GU-CLASP treebank / FraCoq) · **no license** (multifracas data) · original unclear — **eval-only, do not ship** ✗ |
| **lightblue** DTS lexicon | validated *(form, CCG cat, DTS preterm)* triples | **reuse** where present | BSD-3 ✓ (Japanese-strong; English = the gap) |
| **CCGbank** | gold English CCG **categories** | category **eval reference only** | **LDC2005T13 — encumbered** ⚠️ |
| curated closed-class list | function words | the **hardest** categories, hand-seeded from standard categorial treatments | hand-authored |

Three precisions: **WordNet drives only the *content-word* track** (it is content-word-centric;
function words — which carry the compositional weight — are a separate curated track). **WordNet
itself carries verbs** — synsets, the troponym/hypernym hierarchy, *and* coarse **sentence frames**
(transitivity) — so a verb's *category and hierarchy* come from WordNet too; **VerbNet's role is the
narrower one** of refining each argument *slot's type* (selectional restrictions → a hypernym-lattice
class) beyond the frame's bare shape (§8.7.4). And **CCGbank's gold categories are LDC-encumbered** →
eval reference, *not* a shipped dependency; English categories come from **LLM-propose-then-validate**,
not CCGbank reuse.

### 8.2 Work-list (order of generation)

- **Tier 0 — function words** (~few hundred, closed class): hand-seeded categories,
  **FraCaS-validated**, human-heavy. Hardest and most reused → first.
- **Tier 1 — high-frequency content words**: verbs via **VerbNet**, nouns/adj/adv via
  **WordNet**; LLM-proposed-then-validated.
- **Tier 2 — the long tail**: WordNet + LLM, lighter validation.

### 8.3 The loop (per item) — gated and graded

Runs as a kernel-dispatched **institution** (each entry a Derived witness; the battery is the D61
CQ-runner). Validated entries + batteries **compound** (retrieve-first finds them next time).

0. **Retrieve-first** — already-validated entry (lightblue / prior-codified)? reuse, skip.
   *(Source data pinned, content-hashed = Observed.)*
1. **Assemble the seed** — WordNet sense+gloss+hypernyms (+ VerbNet frames/roles/restrictions/
   predicates for verbs) + candidate Eigenius IRIs. Structured context, *not* free generation.
2. **Propose** *(LLM, constrained decoding, k candidates)* — emit `(category, meaning-term,
   grounding)` under the typed-shape schema, seeded by step 1. → **Derived (untrusted)**.
3. **Soundness gate — oracle #1** — type-check each candidate in EigenTT (reuse lightblue/DTS's
   checker, the "Semantic Felicity Condition"). Reject ill-typed.
4. **Build the battery** — LLM-generated **labelled** examples (the *shippable* battery) targeting
   the construct's failure dimensions (negation / scope / plurality / intensionality) **plus
   negatives**, from an **independent** prompt/model; cross-checked against **FraCaS as a
   non-embedded eval reference** where it covers the construct (FraCaS is not shippable — §8.5).
5. **Faithfulness gate — oracle #2** — parse + type + infer each example against the candidate
   (+ rules + lexicon); the derived term must yield the **expected label** (entailment via the
   prover; negatives must fail). Score by pass-rate. → **Derived (validated)**.
6. **Select + human spot-check** — top candidate + a sample of its examples (esp. failures /
   disagreements) → human sign-off. → **Verified**.
7. **Codify** — commit the entry as a typed lexical resource; **store its battery as a permanent
   regression test**.
8. **Regression gate** — re-run affected prior batteries; a new entry that breaks a prior one is a
   **fail-closed finding**. The lexicon grows monotonically-sound.

**Grade ladder per entry:** Observed (pinned source) → Derived (LLM proposal) → Derived (battery
passes) → Verified (human sign-off). The LLM is used only where it is reliable — generating
entries *and* labelled examples (its language strength); both formal gates and the human boundary
do the judging.

### 8.4 What this buys — and the residual

Because an entry's **category is its compositional contract** and composition is the small
universal rule set (§3), a sentence's meaning is the **homomorphic, type-driven composition** of
its entries (DTS/lightblue's design). So a *sound + faithful lexicon yields faithful composition
via the type system* — there is **no separate "composition mis-fires" failure mode**. The
sentence-level residual is therefore **not composition** but:
- **selection / disambiguation** — among the several *well-typed* readings the type system admits
  (sense, scope, attachment), did the parser pick the *intended* one? (Checkable by
  derivation-ranking / the multi-candidate comparison.)
- **coverage** — non-compositional phenomena (idioms, MWEs, constructions) and missing entries.

So validating the lexicon does most of the work; the sentence-level faithfulness check (§5 / D61
oracle #2) shrinks from "did the meaning compose?" to "did we select the intended reading, and is
it covered?" — a far thinner, sharper target.

### 8.5 Open risks
- **FraCaS is not permissively usable** (verified): the GU-CLASP / Gothenburg forms are
  **GPL-3.0** (copyleft), the `multifracas` data carries **no license**, and the original is
  unclear — so, like **CCGbank** (LDC-encumbered), it is **eval-only, never embedded/shipped**.
  Route-around: the loop's shippable battery is **LLM-generated labelled examples** (ours to
  license); FraCaS is only a non-redistributed internal benchmark. (Private benchmarking is use,
  not redistribution; confirm the no-license `multifracas` case before relying on it.)
- **lightblue's English maturity** — this loop is also *how the English DTS lexicon gets built*,
  but that is a real undertaking, not free reuse.
- The example **labels are themselves LLM output** — human-sample them; prefer gold (FraCaS) where
  it covers the construct.

### 8.6 Realized — the lexicon layer + composition + the commit-time felicity check (witnessed)

A first slice of §8 is built and kernel-validated — confirming the central D62 claim at the
smallest scale: *we did not need a new term language; the kernel's `Exp` already is one, and its
Eigon extensions are a lexical-semantics toolkit.*

- **The lexicon layer** — `experiments/lexicon/lexicon.esl`, witnessed by
  `kernel/tests/lexicon_validates.rs` (compiles against core→reflection(+eigentt); `Validator`
  reports 0 errors). The four categorial archetypes each map onto an existing kernel constructor:
  - common noun (`N`) → a **type**: `EigonClass` — CN-as-type is the kernel's *native* model, not
    a fork we add (Luo/Cooper, §3);
  - named entity (`NP`) → a **witness by reference**: `ResourceRef`;
  - transitive verb / adjective (`(S\NP)/NP`, `N/N`) → a **predicate**: `EigonAxiom` — a typed
    chain constant, not an invented proposition symbol.
- **The category is an inductive**, not a string —
  `data lexicon:Cat { cat_s, cat_n, cat_np, fwd(Cat,Cat), bwd(Cat,Cat) }`, carried as a
  kernel-checked `type_expr` term. This is what makes the homomorphism `⟦cat⟧ → sem_type` a
  **recursor** — the hinge that makes the felicity check mechanizable rather than prose.
- **Composition = the kernel type-checker as the felicity oracle** — a well-typed composition
  type-checks; an argument-swapped one is **rejected** (`felicity_filter_*` run the *identical*
  pipeline differing only in argument order, so the rejection is provably the type-checker). The
  Semantic Felicity Condition (§2 stage 5), demonstrated end-to-end.

**Two findings about *where the check fires* (witnessed against the kernel):**
1. **Storage ≠ check.** A proposition *stored* in a `type_expr` field is lowered + D47-encoded,
   not type-checked. The felicity check fires only when a term is routed through the checker — so
   the engine's stage-5 "check" is an **explicit** step, not a side effect of storing the tree.
   At commit it is now the kernel's job: **D49's Rule 21** (`check_type_expr_well_typed`) decodes
   + `check_infer`s *every* `eigentt:TypeExpr`-valued slot, so a committed proposition is
   type-checked, not merely decoded. (That rule consolidated three overlapping eigentt checks into
   one type-system-driven validator — see D49 §6.)
2. **Named entities are not free variables.** A `ResourceRef` in a program body lowers to an
   unbound `Var` in the checker — chain entities need **explicit binding/resolution** when a
   composition is checked. NP references (→ committed chain resources) must be embedded as
   `EigonResource` or resolved against the layer, not handed to the checker as bare `Var`s.

**Realized — the `⟦·⟧` recursor + the mechanized felicity invariant.** `⟦·⟧ : Cat → Type` is built
(`denote_cat`, witnessed by `kernel/tests/lexicon_validates.rs`): `⟦cat_s⟧ = Prop`, `⟦cat_n⟧ = Set`,
`⟦cat_np(T)⟧ = T`, `⟦A/B⟧ = ⟦A\B⟧ = ⟦B⟧ → ⟦A⟧`. The **schematic-atom** problem is resolved by
**type-indexing the entity atom** — `cat_np(T)` carries its class (Luo's DCG move, §3), so `⟦·⟧` is
self-contained (no external atom→type environment needed). Mechanically, `cat` became
`data lexicon:Cat : Type 1 { … cat_np : Set -> Cat … }` (the universe bumps to `Type 1` because the
inductive now stores a `Set`), and `cat_denotation_matches_sem_type` asserts `⟦cat⟧ = sem_type` for
every entry — `cat` is now the checked source of truth, and the homogeneity / argument-order
inconsistency the bare-atom spike hid is forbidden (`denotation_is_order_and_type_sensitive`).
Conventions settled: transitive verbs are **object-first** (`⟦(S\NP)/NP⟧ = ⟦obj⟧ → ⟦subj⟧ → Prop`);
the adjective is given **predicatively** (`S\NP`), with the attributive `N/N` (Σ-refinement → a class)
left to a type-shifting composition rule.

**Realized — the composition parser (stage 2).** A CKY chart over the categorial `cat`s
(`kernel/tests/lexicon_validates.rs`): each step combines two items by forward/backward
application — on the *category* (`fwd`/`bwd`) and, in lockstep, on the *sem* (`App`) — and the
kernel confirms the assembled term. `parser_composes_sentence_to_checked_prop` parses *"HeLa
depends on BRCA1"* → exactly one `S` parse whose assembled sem `depends_on(brca1, hela)`
**type-checks to `Prop`** (the felicity of the *whole* sentence, kernel-confirmed) — the first
prose-tokens → EigenTT-term → kernel-check loop. `parser_rejects_type_mismatched_sentence` parses
*"BRCA1 depends on HeLa"* (subject/object types swapped) → **no `S` parse**: the categories don't
combine, so the felicity filter fires *at the category level*, before any sem is assembled. Named
entities are resolved to values (`resolve_sem`: axiom → `EigonAxiom`, class → `EigonClass`,
instance → `EigonResource`), closing the earlier "chain entities are not free variables" finding.

**Realized since:** the engine is **extracted from the test harness into the `kernel::dcg` module**
([`kernel/src/dcg/`](kernel/src/dcg/), the dependent categorial grammar engine, broken into
`category` / `parser` / `lexicon` / `lemmatizer` components with a flat public re-export) —
`denote_cat`, the combinator, CKY, plus **`gate_entry`** (the callable felicity gate) — and exposed
as the CLI **`eigenius lexicon gate`** (chain-loads schema + entries; admit/reject per entry,
fail-closed). **CN-as-types subsumption** is wired: `Layer::is_subclass_of` (the single foundation
authority) + the `EigonClass` subtype rule in `nbe::check` honor `core:subclass_of` as subtyping
(the inclusion-coercion fragment of Luo `luo2012coercive`), so a general predicate typed at a
supertype accepts subclass-typed arguments (witnessed: a general verb `affects : Entity → Entity →
Prop` composes with `Gene`/`CellLine` arguments).

**Realized since — the deterministic WordNet → lexicon mapper (the general English framework, §8.7).**
Built as a standalone crate [`crates/eigenius-wordnet`](crates/eigenius-wordnet/) (the
`wordnet-import` binary; lib `wndb` reader + `convert` mapper + the Morphy port), **not** part of the
`eigenius` CLI. Run on the **full WordNet 3.0 corpus** (`--all`, noun+verb+adj) it emits the general
lexicon — **74,385 noun classes** (the `@` hypernym graph → the `core:subclass_of` lattice, rooted at
`entity.n.01`), **7,730 proper-noun individuals** (the `@i` instance synsets → `EigonResource`s, the
NP archetype — §8.7.3), **33,006 verb/adjective axioms**, and **204,088 `lexicon:LexicalEntry`
resources** — and self-checks fail-closed (`--validate`: compile + `Validator` + `gate_entry`).
WordNet's **Morphy** is ported faithfully ([`morphy.rs`](crates/eigenius-wordnet/src/morphy.rs)) as
the `Lemmatizer` reference impl for the lookup stage (§8.8). The **LLM proposer** was prototyped and
run end-to-end (the *augmentation / domain-binding* path, §8.7.8), but those orchestration changes
are **discarded for now** — the deterministic base is the foundation; the augmentation layer is
rebuilt on top of it later.

**Still ahead:** the rest of the **parse pipeline** (§8.8) — the **lookup bridge is realized**
([`kernel::dcg::lookup`](kernel/src/dcg/lookup.rs): the form-keyed index + multi-span lemmatized
seeding + `parse`, Morphy-driven end to end, §8.8.5); what remains is the encoding institution (the
FIBER-INTO generation query + the AutoOnLoad felicity gate + the *INTO-opts-into-AutoOnLoad* kernel
hook) and the selector + alternative-recording; **VerbNet** argument-type refinement (§8.7.4); broader grammar
(ambiguity + derivation-ranking, type-raising, coordination, clausal / control frames); the
attributive-adjective type-shift; and an *in-kernel* `⟦·⟧` recursor (a large elimination — engine-side Rust
suffices for now).

### 8.7 The WordNet → lexicon mapper — complete specification

The general English lexicon is **imported from WordNet's structure by a deterministic mapper,
kernel-gated** — not LLM-proposed. WordNet already encodes the three things a typed categorial
lexicon needs — **synsets** (the types and predicates), **hypernymy** (the subclass lattice), and
**sentence frames** (the categories) — so the bulk of the import is a structural transform, and the
LLM loop (§8.3) is reserved for genuine judgment (function words, argument-type refinement, sense
selection, domain augmentation). This is the inversion of the earlier framing: **the general
framework is the foundation; domain-specific vocabulary is an additive layer** (§8.7.8). It is also
exactly the CN-as-types-from-WordNet construction of the prior art (Luo, `luo2012cnt`; §3).

#### 8.7.1 Source & record format

WordNet 3.0 `data.<pos>` / `index.<pos>` (`wndb(5WN)`), `pos ∈ {noun, verb, adj, adv}`. Each synset
is one `data.<pos>` line:

```
offset lex_filenum ss_type w_cnt (word lex_id)+ p_cnt (ptr_sym offset pos src/tgt)* [f_cnt (+ f_num w_num)*] | gloss
```

Beyond lemmas + gloss, the reader ([`crates/eigenius-wordnet/src/wndb.rs`](crates/eigenius-wordnet/src/wndb.rs))
captures the two structural fields the mapper needs: the **pointer records** (esp. `@` hypernym,
`@i` instance-hypernym), and — verbs only — the **frame field** (`+ f_num w_num`). License: the
WordNet license is permissive/OSI — shippable.
Offsets are **version-specific**, so the mapper pins a WordNet version and records it in provenance.

#### 8.7.2 Identity

- **synset → class/predicate IRI**: `urn:eigenius:wn:<ver>:<pos><offset>` (version-pinned locator).
  For durable, cross-version identity adopt the **ILI** (Interlingual Index, Open English WordNet)
  as the stable id, with the offset as the within-version locator.
- **lemma → `lexicon:LexicalEntry`** whose `lexicon:sense` is the WordNet **sense key**
  (`lemma%ss_type:lex_filenum:lex_id::`). One synset yields one type/predicate and *N* entries (one
  per lemma); one lemma across senses yields several entries (the parser's forest selects, §8.4).

#### 8.7.3 Nouns → classes (CN-as-types)

- Each noun synset → a **`core:Class`** (the type — CN-as-types, the kernel's native model, §8.6).
  `core:description` = gloss.
- Each `@` / `@i` pointer → a **`core:subclass_of`** edge. The noun hypernym DAG, rooted at
  `entity.n.01`, **becomes the kernel's subclass lattice** — the *same* `core:subclass_of` the
  subsumption rule consumes (`Layer::is_subclass_of` + the `EigonClass` subtype rule in `nbe::check`,
  §8.6). The hand-added `Entity` supertype of the spike was a stand-in for `entity.n.01`.
- **Instance** synsets (`@i`, proper-noun individuals like *Einstein*) → the **named-entity (NP)**
  archetype: an `EigonResource` instance of its class, not a class.
- Each lemma → an entry: `cat = cat_n` (N), `sem` = the synset class, `sem_type = Set` (`⟦cat_n⟧`),
  `sense` = sense key, `grade = Declared`. Multiword lemmas (`take_a_breath` → `"take a breath"`).

#### 8.7.4 Verbs → predicates

- Each verb synset → an **`eigentt:Axiom`** (a typed chain constant — the predicate; the EigonAxiom
  archetype), not an invented proposition symbol.
- **Category from the sentence frames** (the `f_num` field; §8.7.6). A synset may carry several
  frames → several categorial entries (the verb's alternations: e.g. `breathe` has frame 2 → `S\NP`
  *and* frame 8 → `(S\NP)/NP`).
- **Argument types — two stages, bridged by subsumption:**
  - **Stage 1 (WordNet-only):** type each NP slot generically at the noun root — `cat_np(entity.n.01)`
    — so the predicate is `entity → … → Prop`. By subsumption (§8.6) it composes with *any* noun:
    broad coverage, loose felicity. This needs *no* VerbNet.
  - **Stage 2 (VerbNet refinement):** push slot types *down* the hypernym lattice from VerbNet
    thematic roles + selectional restrictions (Agent `+animate` → `animate_thing.n.01`, Patient
    `+comestible` → `food.n.01`), so *"eat a rock"* becomes a type mismatch the gate catches. The
    VerbNet↔WordNet join is VerbNet's `wn=` sense attribute on members.
- **`@` hypernym (troponymy)** is recorded among the axioms as an **entailment** relation
  (*whisper ⇒ speak*), **not** as an `EigonClass` subtype edge — predicate subsumption over function
  types is subtler than class subsumption and is deferred (§8.7.10).
- Each lemma → entries: `cat` from frames, `sem` = the axiom, `sem_type = ⟦cat⟧` (the recursor),
  `sense`, `grade = Declared`.

#### 8.7.5 Adjectives & adverbs

- **Adjectives** (`data.adj`) → the **predicative** archetype (`S\NP[X]`, an `EigonAxiom`). Stage-1:
  typed at `entity.n.01` (`entity → Prop`); refinement via the `=` attribute and `&` similar-to
  (satellite→head) pointers is deferred. The **attributive** `N/N` use (a Σ-refinement → a class) is
  the type-shift already deferred in §8.6.
- **Adverbs** (`data.adv`) → predicate/sentence modifiers (`(S\NP)/(S\NP)`, …); deferred, with the
  `\` pertainym pointer (adverb→adjective) recorded.

#### 8.7.6 Frame → category table

The WordNet sentence frames (1–35) map to CCG categories. The load-bearing subset (frames 2 and 8
confirmed against real `data.verb`):

| Frame | Template | Category |
|---|---|---|
| 1 | Something ----s | `S\NP` |
| 2 | Somebody ----s | `S\NP` |
| 8 | Somebody ----s something | `(S\NP)/NP` |
| 9 | Somebody ----s somebody | `(S\NP)/NP` |
| 11 | Something ----s something | `(S\NP)/NP` |

The mapper carries the **full 35-frame table** (transcribed from the WordNet docs — a verifiable
source; not reproduced in full here). Clausal-complement / control / raising frames (*"Somebody
----s that CLAUSE"*, *"Somebody ----s to INFINITIVE"*) map to **higher-order categories** and are
flagged as the hard tail (they interact with the deferred type-raising grammar, §8.6 "still ahead").

#### 8.7.7 The import algorithm

1. **Parse** `data.<pos>` — extend the reader to capture `@`/`@i` pointer records and the verb frame
   field.
2. **Mint synset resources** — classes (nouns), axioms (verbs/adjs), with `subclass_of` from `@`.
   The hypernym graph is a DAG; the layer chain resolves references, so emission order is flexible
   (or topo-order parents-before-children within a layer).
3. **Mint lexical entries** per `(lemma, synset)` — category from frames (verbs) / fixed (`N` nouns,
   predicative adjs); `sem_type = ⟦cat⟧` derived by the recursor (so `⟦cat⟧ ≡ sem_type` by
   construction, as in the proposer).
4. **Route through the kernel** — classes/axioms validated by the `Validator`; entries by the
   felicity gate (`gate_entry` / `eigenius lexicon gate`). A rejection is a **fail-closed finding**,
   not a silent drop.
5. **Provenance & grade** — record WordNet version, sense keys, ILI, source content-hash; `grade`
   is `Observed` for the pinned source, `Declared` for the mechanical mapping, elevated to
   `Derived`/`Verified` only by the §8.3 battery/human gates.

Scale: ~146k noun synsets, ~25k verb, ~30k adj. The importer emits a large layer (or a per-POS layer
chain); identity/indexing/perf at this scale is an implementation concern (§8.7.10).

#### 8.7.8 Domain augmentation (the additive layer)

A domain ontology binds to the general framework by **`subclass_of` into it** — `bio:Gene
subclass_of` the WordNet gene synset, a domain predicate specializing a general verb's argument types
— after which domain text parses with general + domain entries, the subsumption rule bridging. An
**LLM proposer** is the intended **augmentation tool** for this layer: it maps a domain term onto its
general parent and mints domain predicates, kernel-gated, with a constrained-`--vocab` mode (map a
word onto a fixed domain menu) as the *top* layer over the deterministic WordNet import (this
section) as the *base*. Such a proposer was prototyped and run end-to-end on the real WordNet corpus,
but those orchestration changes are **discarded for now** (§8.6) — the deterministic base is the
foundation; the augmentation proposer is rebuilt on top of it later. The trust seam is the same
either way: an untrusted proposer only ever *proposes*; the kernel admits or rejects.

#### 8.7.9 Trust & the residual

WordNet is **untrusted input**; the kernel is the oracle (every minted resource is validated). The
import is *type-correct by gate*, **not** *faithful by gate*: WordNet's sense granularity, frame
coverage gaps, and the stage-1 generic typing are the residual — the D61 faithfulness concern (§5).
Word-sense ambiguity becomes multiple entries per lemma, resolved by the parser's forest +
derivation-ranking (§8.4), not by the importer.

#### 8.7.10 Open questions / deferred

- **Predicate subsumption** — verb troponymy/entailment is *not* the `EigonClass` subtype rule;
  it needs an entailment relation among axioms. (Class subsumption covers nouns; this is the verb gap.)
- **Multi-class instances — resolved ([#91](https://github.com/eigenius/eigenius/issues/91)).** `@i`
  synsets are emitted as `EigonResource` individuals (§8.7.3 / §8.6); the 786 with **multiple** classes
  carry **all** of them on the resource (`resource r : C1, C2, …`) **and** now get one NP entry per
  class. The kernel's **check-mode resource-inhabitation rule** (full `is_a` × `is_subclass_of`) + the
  **check-mode felicity gate** admit a name at *any* of its classes, including the non-first — so the
  per-class entries gate-pass. (`check_infer(EigonResource)` keeps its `is_a().first()` best-effort:
  it is off the inhabitation path and never load-bearing — see #91 for the narrowed synthesis note.)
- **Identity stability** — offset (version-pinned) vs **ILI** (cross-version); adopt ILI for durable ids.
- **Scale** — ~200k synset-classes: layer size, resolution/indexing, gate throughput.
- **Coverage** — attributive adjectives (Σ), adverbs, multiword expressions / idioms / constructions
  (non-compositional — the coverage track, §8.4); verbs absent from VerbNet stay generically typed.

### 8.8 The parse pipeline — the string→tree(s) library and the encoding institution

§8.6 realized the engine's *internals* (`denote_cat`, the CKY combinator, `gate_entry`) as the
kernel-attached `kernel::dcg` module; §8.7 imports the *lexicon data*. This section fixes the
**runtime architecture** that turns a prose statement into a committed, referenceable EigenTT tree,
and resolves the four design questions that surfaced once the engine was real. The shape is a
**two-layer split** — a kernel-attached **library** that maps a string to the *forest* of
type-checking parses, wrapped by an **encoding institution** that selects one parse and commits it
as a `lexicon:Sentence` resource for objectives and witnesses to reference.

| # | Question | Resolution |
|---|---|---|
| **Q1** | What does the library produce vs. the institution? | library → the *forest* (unanchored, transient); institution → exactly one committed `lexicon:Sentence` (encoding-task-anchored) — §8.8.1–8.8.2 |
| **Q2** | Who selects among an ambiguous forest? | the **institution** selects (the faithfulness step) and **records the discarded alternatives**; the library never selects — §8.8.3 |
| **Q3** | How does the institution commit the result? | a **commit-capable `FIBER … INTO` query** (OnDemand) generates + commits; an **AutoOnLoad** role felicity-gates the committed resource (fail-closed) — §8.8.2 |
| **Q4** | Where does format cleaning/normalization live? | plain text first (library + institution alone); format cleaning is a **separate D60 tool-runtime pre-stage**, never baked into the trusted library or the gate — §8.8.4 |

#### 8.8.1 The library — string → forest (Q1, lower half)

`kernel::dcg` is the **kernel-attached library**: the trusted, deterministic compositional engine,
the felicity *oracle*. Its parse entry point is a pipeline over the realized components —
`lemmatizer` → lexicon lookup → `parser` (CKY) → felicity-checked forest:

1. **Tokenize + lemmatize.** Each surface token is reduced to its base lemma(s) via the
   [`Lemmatizer`](kernel/src/dcg/lemmatizer.rs) seam (WordNet's Morphy in `eigenius-wordnet` is the
   reference impl; `Identity` the baseline). Morphological ambiguity (`axes → {axe, axis}`) becomes
   *multiple* candidate lemmas, hence multiple leaf items.
2. **Lexicon lookup — the bridge** ([`LexicalIndex`](kernel/src/dcg/lookup.rs)). A `form → entries`
   index over the imported lexicon (§8.7), each entry pre-resolved to a parse `Item`; POS keys the
   *lemmatizer* (every part of speech is tried per span), the index itself is form-keyed and
   case-insensitive. Lookup is **multi-span**: for each token span (bounded by the longest indexed
   form) the surface is reduced to candidate lemmas and looked up, so a multiword entry
   (`take a breath`, the `act on` collocation) seeds an item spanning several tokens *alongside* the
   single-token items for its parts (`on` keeps its own entries) — MWE-vs-compositional carried as
   competing chart edges (append-not-overwrite), not resolved here.
3. **Compose.** The chart (built by [`LexicalIndex::parse`](kernel/src/dcg/lookup.rs) over the
   seeded spans) combines items by forward/backward [`apply`](kernel/src/dcg/parser.rs) on the
   `lexicon:Cat`s, in lockstep on the `sem` terms; each complete `S` parse whose assembled `sem`
   type-checks to `Prop` (the kernel as oracle, §8.6) is a forest member.

The library returns the **whole forest** — *every* well-typed parse — as **transient terms**
(unanchored `Exp` values), with **no selection and no commit**. This is the literal reading of the
user's "string → tree(s)": *trees*, plural, on purpose. Ambiguity (MWE vs. compositional, word
sense, attachment) is exactly §8.4's *selection residual*, surfaced as forest cardinality rather
than hidden. An **empty** forest is a first-class outcome (no admissible parse), not an error to
swallow.

#### 8.8.2 The encoding institution — statement → one committed resource (Q1 upper half, Q3)

The institution wraps the library *for the encoding task*: a textual statement carried by an
objective or a witness's prose becomes exactly **one** `lexicon:Sentence` resource, anchored in the
chain and referenceable downstream. It is the §6 generation institution, made concrete. The two
capabilities are the user-specified Q3 answer — **a commit-capable fiber query, and an AutoOnLoad
gate** — and both are grounded in existing kernel mechanism:

- **Generation = `FIBER … INTO` (OnDemand).** The institution surfaces the parse as a query
  `FIBER <prose> USING INSTITUTION <encoding> INTO "<sentence-iri>"`: it runs the library, selects
  (§8.8.3), and the `INTO` clause **chain-commits** the chosen `lexicon:Sentence` through the
  `CommitOrchestrator` under `WithRetroactive` ([server/query.rs:91](kernel/src/server/query.rs#L91),
  [:153](kernel/src/server/query.rs#L153); D14 §9.3 chain-reinsertion). The kernel **requires** a
  FIBER QueryClass to carry the **OnDemand** dispatch role
  ([type_check.rs:539](kernel/src/query/type_check.rs#L539): *"FIBER query class … has no OnDemand
  dispatch role — declare on_demand … to allow FIBER dispatch"*). So generation is on-demand **by
  the kernel's own contract** — you run the query to encode a statement; the engine never gates
  arbitrary commits. This is precisely §6's "the engine generates; it does not admit," now mechanized.
- **Felicity gate = AutoOnLoad (fail-closed).** The committed `lexicon:Sentence` /
  `lexicon:LexicalEntry` classes carry an **AutoOnLoad** QueryClass (`result_class = Verdict`) that
  runs the felicity check (`gate_entry` / the sentence's assembled-term type-check, §8.6) when the
  resource loads. A `Fails` verdict becomes an `AutoOnLoadOutcome` error and the load is **rejected**
  ([institution/dispatch.rs](kernel/src/institution/dispatch.rs)). The §8.6 *commit-time* felicity
  check is thereby realized as a D14 institution gate: structural, deterministic, fail-closed — and
  distinct from faithfulness (it certifies the tree type-checks, never that it means the prose).

**The one kernel touch-point (named, not assumed).** The FIBER-INTO commit path **deliberately
bypasses AutoOnLoad** today — *"the commit path deliberately bypasses AutoOnLoad until INTO opts back
in"* ([server/query.rs:106](kernel/src/server/query.rs#L106)). So the felicity gate does **not** fire
automatically on a FIBER-INTO-committed sentence as the code stands. Closing this is the anticipated
*"INTO opts back in"* hook: the encoding institution's INTO surface opts its committed
`lexicon:Sentence` into AutoOnLoad so the gate runs. This is a small, identified extension of the
INTO surface — recorded here as the active gap rather than papered over with an assumption that the
gate already fires.

The emitted `lexicon:Sentence` is **Derived** by construction — an institution dispatch yields a
`DerivedResource` under a `ProgramTrace → IsDerivedAs` (§6, D56) — i.e. provisional until D61's
faithfulness check + the human boundary climb its grade (§5). Never auto-Verified.

#### 8.8.3 Selection and the recorded alternatives (Q2)

The library returns the forest; the **institution selects** the intended parse. This is the
faithfulness step, and it must be honest about its own limits: the type system guarantees every
forest member is *well-typed*, never which one matches the author's intent — MWE-vs-compositional
(`act on` the collocation vs. `act` + PP), word sense, and lexical-choice slack
(`regulate`/`inhibit` → `affects`) all leave ≥1 well-typed reading. Selection is therefore exactly
where the D61 faithfulness concern lives (§5, §8.4), not a detail the engine settles silently.

- **Fail-closed, auditable.** A **single** forest member commits directly. **Multiple** → the
  institution selects one and **records the discarded parses as provenance** on the `lexicon:Sentence`
  (the alternative trees + the selection warrant) — never a silent drop. An **empty** forest is a
  **fail-closed finding** ("no admissible parse for ⟨prose⟩"), surfaced for investigation, not
  skipped.
- **Staged policy.** Deterministic preference first (longest-MWE, sense priors, derivation cost);
  the D61 faithfulness oracle (LLM-judge + human, → Derived/Verified) for the residual. The selector
  is a ranking *over an already-felicitous forest*, so a wrong selection is a faithfulness miss, never
  a type error — the two failure modes stay cleanly separated.

#### 8.8.4 Input cleaning and format (Q4)

v1 is **plain text**: the library + institution alone handle clean prose (statement string → forest
→ one resource). Format-specific cleaning and normalization — de-markup, sentence segmentation,
quote/unicode normalization, source-format extraction — is a **separate pre-stage**: a D60
tool-runtime component run *before* the institution, **never** baked into the trusted library or the
felicity gate. Keeping it out preserves the library's determinism and trust (the same
generation-vs-cleaning separation the schema.org generator's format front-ends already use, §6
substrate D26/D56/D60); the encoding institution simply consumes the cleaned text. Format handling
is thus additive and deferred, not a v1 dependency.

#### 8.8.5 Status

**Realized — the lookup bridge** ([`kernel::dcg::lookup`](kernel/src/dcg/lookup.rs)): the form-keyed
[`LexicalIndex`](kernel/src/dcg/lookup.rs) over the committed lexicon, multi-span lemmatized seeding,
and the `LexicalIndex::parse(&str, &dyn Lemmatizer) → Vec<Item>` entry point that returns the forest
of full-span `S` parses the kernel types to `Prop`. Witnessed in `kernel/tests/lexicon_validates.rs`
(MWE-verb sentence → one `S`-to-`Prop`; general verb via subsumption; case-insensitivity; empty
forest for unknown words; no parse for a type-mismatch) and — driven by the real **Morphy**
`Lemmatizer` ([`eigenius-wordnet`](crates/eigenius-wordnet/src/lemmatizer.rs)) — in
`crates/eigenius-wordnet/tests/morphy_bridge.rs` (an *inflected* sentence parses only because Morphy
reduces it to the base entry; an `Identity` control yields no parse, isolating the morphology's role).

**Still ahead:** **(i)** the **encoding institution** — the FIBER-INTO QueryClass (OnDemand) + the
AutoOnLoad felicity QueryClass, plus the **INTO-opts-into-AutoOnLoad** kernel hook (§8.8.2);
**(ii)** the **selector** + alternative-recording provenance (§8.8.3). The format-cleaning component
(§8.8.4) and the D61 faithfulness oracle are separate, later work.

## 9. Prior art / anchors (to verify via the §4 grounding pass)
- Cooper, R. *From Perception to Communication: A Theory of Types for Action and Meaning.* OUP,
  2023 (open access; DOI 10.1093/oso/9780192871312.001.0001) — **TTR**, the records-first
  substrate match (§3); primary-read.
- Carpenter, B. *Type-Logical Semantics.* MIT Press, 1997 — the formal spine (§3).
- Luo, Z. *Common Nouns as Types* (LACL 2012) / *Formal Semantics in MTTs with Coercive
  Subtyping* (2012) — the MTT/entities-as-types half (shared with D61/D18).
- Chatzikyriakidis, S.; Luo, Z. *Formal Semantics in Modern Type Theories.* ISTE/Wiley, 2020
  (DOI 10.1002/9781119489252) — the comprehensive MTT-semantics reference (both model- and
  proof-theoretic; Coq-verified; impredicative `Prop` ≈ D46). Main chapters paywalled.
- Steedman, M. — Combinatory Categorial Grammar (the CCG lineage).
- Fillmore, C. — FrameNet / frame semantics (valence).
- Caufield et al., *SPIRES* (Bioinformatics 2024) — schema-constrained extraction (shared with
  D61 §10/D50).
- The autoformalization faithfulness sources (D61 §10: Herald, miniF2F-Lean Revisited, ReForm)
  — why stage 5 is mandatory.

(Bibliographic details to be verified in the grounding pass before any are committed as
load-bearing anchors — never fabricate.)

## 10. Out of scope
- Open-domain autoformalization treated as solved — it is the bottleneck, not a primitive.
- The general HOL→EigenTT translation as a finished theory — §3's gap is real research; D62
  scopes a domain-bounded slice, not a universal translator.
- Running the engine without D61's check — definitionally excluded (the engine is the untrusted
  step).
