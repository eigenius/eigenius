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
- **Dispatch role = on-demand, not a gate.** The engine *generates*; it does not *admit*. It is
  an **on-demand** institution (D31) you invoke to encode a piece of prose — never AutoOnLoad (it
  is not a commit gate). This makes the engine/check split a clean **generation institution (D62,
  Derived) + verification institution (D61's faithfulness oracle, Verified/Fails)** pair — two
  institutions in one framework, mirroring how the reasoning institution verifies what other
  producers emit.

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

Each slice ships **with** D61's check, never before it:
1. **Slice 1 (first real value):** extend **D8 `CompleteJson`**, schema-constrained to the D61
   `verify:` / typed decision vocabulary, frame-assisted (FrameNet/VerbNet valence as the slot
   guide) — emitting *candidate* trees straight into D61's check. No type-logical core yet; the
   LLM proposes, D61 checks.
2. **Slice 2:** the type-logical composition core (stage 2) — categorial derivation as the
   compositional check, replacing free-form LLM structure with a checked one.
3. **Slice 3:** broader parsing front-ends + the HOL→EigenTT target mapping (stage 4) at depth.

## 8. Bootstrapping the lexicon

The lexicon (stage 1) is the engine's **bottleneck** — and the only genuinely new linguistic
resource (the composition rules are a small universal set; §3). It is **bootstrapped from existing
permissive resources by an LLM-proposer loop, validated formally, and codified at a graded
witness** — never hand-built, never trusted unchecked. Because compositionality is lexicalized in
the type system (§8.4), validating the lexicon largely validates composition too.

### 8.1 Inputs — existing data, per entry-field + license

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

Two precisions: **WordNet drives only the *content-word* track** (it is content-word-centric;
function words — which carry the compositional weight — are a separate curated track). And
**CCGbank's gold categories are LDC-encumbered** → eval reference, *not* a shipped dependency;
English categories come from **LLM-propose-then-validate**, not CCGbank reuse.

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
