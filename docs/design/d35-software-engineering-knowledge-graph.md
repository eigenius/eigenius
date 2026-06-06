# D35 — Software Engineering as Knowledge Graph

*Status: draft proposal · May 2026*

*Companion documents: [D3 program model](d3-program-model.md), [D14 institution realisation](d14-institution-realisation.md), [boundary contracts](boundary-contracts.md), [D6 execution architecture](d6-execution-architecture.md), [D22 notebook and TypeScript SDK](d22-notebook-and-typescript-sdk.md), [D24 schema versioning](d24-schema-versioning.md).*

---

## 1. Motivation

AI coding agents today operate against codebases through text retrieval. They `grep`, they read files, they synthesise — and they reconstruct, on every task, the structural understanding that the project's contributors carry implicitly. The retrieval is grounded only in lexical proximity; the reasoning is grounded only in what fits in a context window. The agent does not know that a function is the realisation of a particular design element, that a test asserts a particular requirement, that a module sits behind a particular boundary contract, or that a recent commit invalidated a previously-derived guarantee. It infers these things, sometimes correctly, from prose.

Eigenius already provides the substrate to do better. Its core commitment — that every fact carries provenance, that derivations are typed and replayable, that epistemic categories are first-class — is exactly the substrate a coding agent needs to work against a codebase the way an experienced contributor does: structurally, with awareness of intent, contracts, coverage, and history.

This document proposes that Eigenius itself be modelled in Eigenius. The codebase, the design corpus, the test suite, the user guides, and the requirements behind them are all loaded as typed Resources in a dedicated ontology (`urn:eigenius:se`). Coding agents — operating through the existing TypeScript SDK and notebook surface — query this knowledge graph for context, propose changes against it, and emit reasoning traces back into it.

The proposal is deliberately a *hybrid*. For most of the codebase the graph is a *bidirectional index*: the source files are authoritative, the graph is derived from them, and agent edits flow source-first with the graph re-derived on commit. For a small set of schema-shaped artifacts — boundary contracts, the ESL grammar, the EigenQL surface, the gRPC API, the Eigon serialisation shape — the graph is the *source of truth* and code is generated from it. The hybrid plays to Eigenius's strengths without demanding the kernel rewrite that a full model-driven approach would require.

## 2. Scope

In scope:

- A new ontology namespace `urn:eigenius:se` defining classes for software-engineering concepts (Requirement, DesignElement, Module, CodeArtifact, TestCase, Doc, ChangeSet, …) and the relations among them.
- An ingestion architecture that lifts the Eigenius repository (Rust 82.6%, TypeScript 12.3%, Julia 3.4%, Python, Shell, Bicep) into the graph, with one Component per language and one Layer per commit.
- A small set of validation institutions (Lint, TypeCheck, TestRunner, optionally ProofCheck) that fire as `QueryClass(AutoOnLoad)` when code Resources enter the chain.
- A selective model-driven slice in which boundary contracts and a handful of grammar-and-protocol artifacts are *authored* in ESL and *generated* into the source tree.
- An agent-integration story riding on the existing TypeScript SDK and notebook UX described in D22.

Out of scope:

- Replacing git. The repository remains the canonical store for source bytes; the graph holds typed claims *about* those bytes.
- Replacing the existing build system. Cargo, `tsc`, Pkg.jl, `pip`, `bicep build`, and the like continue to drive compilation; institutions wrap their outputs.
- Modifying the kernel. The proposal lives entirely in ontology, Components, and institutions registered through the existing extension surface.
- A workflow engine for agents. As D3 establishes, Programs are dependent functions, not DAGs of tasks. Agent operations are Programs whose Components are tools.

## 3. The `urn:eigenius:se` ontology

### 3.1 Entity classes

The ontology declares a small, opinionated set of classes. Each is a normal `Class` resource with the usual `requires` / `recommends` declarations. None of them needs new kernel mechanisms; all of them ride on the standard Resource / Property machinery. Where this document references the existing boundary-contract vocabulary, it does so under the proposed namespace `urn:eigenius:contracts` — that namespace prefix is itself a proposal of this document, since `boundary-contracts.md` does not pin an IRI for the Class definitions it introduces.

Resources of each class conform structurally to one of the four base epistemic classes (`DeclaredResource | ObservedResource | DerivedResource | VerifiedResource`) introduced in architecture-v0.3 §1; the "Epistemic default" column below records the conformance the SE ontology expects.

| Class | Description | Epistemic default |
|---|---|---|
| `se:Requirement` | A statement of intent — what the system must do, must not do, or must guarantee. | Declared |
| `se:DesignElement` | A named piece of architectural intent: a component, a subsystem, an algorithm, a wire format. | Declared |
| `se:Crate` / `se:Package` | A unit of deliverable code (a Cargo crate, an npm package, a Julia package, a Python distribution). | Observed |
| `se:Module` | A namespace within a Crate — a Rust `mod`, a TS module, a Julia module. | Observed |
| `se:CodeArtifact` | A named, addressable construct in source: a Rust `fn` / `struct` / `trait` / `impl`, a TS `function` / `class` / `interface`, a Julia function / type, a Python function / class, a shell function, a Bicep resource declaration. Specialised by subclass per kind. | Observed |
| `se:TestCase` | An executable assertion. Includes Rust `#[test]`, integration tests, TS test cases, Julia `@test`, Python tests, shell test scripts. | Observed |
| `se:Doc` | A documentation artifact: a markdown chapter from `guides/`, a design document from `design/`, a Rustdoc comment, a TSDoc comment. | Observed (for repo files) / Declared (for hand-written narrative). |
| `se:ChangeSet` | A coherent set of edits proposed or committed to the repository. Wraps a git commit or a not-yet-committed agent proposal. | Observed |
| `se:AnalysisResult` | The output of a derivation Component (lint, typecheck, test run, coverage measurement). | Derived |
| `se:VerifiedProperty` | A claim about a `CodeArtifact` accompanied by a checked proof term (Verus on Rust, Lean 4 via D28, Julia formal-methods bridges via D27). | Verified |
| `se:Author` | A first-class actor record — a human contributor or an AI agent identity — used as the range of `se:authored_by`. The SE ontology introduces this class because no existing actor class is documented in the corpus. | Declared |
| `se:Intent` | A first-class record of an *agent's* plan: what it intends to do, against which Requirements, with which expected effect on which CodeArtifacts. Distinct from the resulting ChangeSet. | Declared |

Subclasses of `CodeArtifact` are introduced per language — `se:RustFunction`, `se:RustTrait`, `se:RustImpl`, `se:TsFunction`, `se:TsClass`, `se:TsInterface`, `se:JuliaFunction`, `se:PythonFunction`, `se:ShellFunction`, `se:BicepResource` — each with the language-specific `requires` (e.g. `se:RustFunction requires se:fn_signature, se:abi`). The decomposition is shallow on purpose: deeper structural detail (the AST itself) lives behind extractor Components, not as schema.

### 3.2 Relations

The relations carry the structural meaning the graph exists to make queryable. They are properties whose `class_types` constrain their domain and range.

| Property | Domain → Range | Reading |
|---|---|---|
| `se:realizes` | `CodeArtifact → DesignElement` | "This function/type/module is part of how this design element is implemented." |
| `se:satisfies` | `DesignElement → Requirement` | "This design element exists to meet this requirement." |
| `se:asserts` | `TestCase → Requirement \| DesignElement \| CodeArtifact` | "This test witnesses this claim." |
| `se:covers` | `TestCase → CodeArtifact` | Coverage in the operational sense (the test exercised this code on its last run). Derived from coverage Components. |
| `se:depends_on` | `CodeArtifact → CodeArtifact` (and `Crate → Crate`) | Direct dependency. Transitive closure is an EigenQL recursive rule, not a property. |
| `se:declared_in` | `CodeArtifact → Module → Crate` | Lexical containment. |
| `se:contracted_by` | `CodeArtifact → contracts:BoundaryContract` | Bridges into the proposed `urn:eigenius:contracts` namespace. |
| `se:documented_by` | `CodeArtifact \| DesignElement → Doc` | "This narrative explains this thing." |
| `se:changed_in` | `CodeArtifact → ChangeSet` | Edit history. |
| `se:authored_by` | `ChangeSet \| Intent → se:Author` | Attribution to human contributors or AI agents. |
| `se:results_in` | `Intent → ChangeSet` | An agent's plan and the change it ultimately produced. |

Note the deliberate absence of an `se:transitively_depends_on` property. The platform deliberately excludes transitive properties from the schema (architecture-v0.3 §3.9 — the Decidability Boundary; OWL union/intersection are admitted only as registered capabilities, never as built-in inference). Closure queries belong in EigenQL `DEFINE` rules, not in the ontology.

### 3.3 Boundary contracts as the spine

The existing `boundary-contracts.md` defines `BoundaryContract` (which holds `declared_operations` and `error_taxonomy`, per §6.1–6.2) and `OperationContract` (which carries the per-operation fields: `input_type`, `output_type`, `effects`, `determinism_class`, `idempotence_class`, `preconditions`, `postconditions`, `resource_bounds`). It also defines the `ImplementationManifest` mechanism with content-hashed `ImplementationComponent`s tagged `Trusted | Advisory | NonTrusted` (§6.7), and the spectrum of formalisation — Documentation / Schema / Runtime / TypeLevel / ProofLevel (§4.1–4.5) — that each clause of a contract may sit at. This vocabulary already says most of what we want to say about a public function or a module API. The SE ontology should not duplicate it; it should *bridge into* it via `se:contracted_by`.

The formalisation spectrum is particularly important for agents. When an agent proposes a change to a contracted boundary, the formalisation level of each affected clause determines what additional artifacts the change must touch — a TypeLevel clause demands a corresponding type-system update; a ProofLevel clause demands a corresponding proof artifact; a Documentation clause demands only that the prose stay coherent.

### 3.4 Where intent lives

`se:Requirement` and `se:DesignElement` are the platform's existing answer to "where do we record what the system *should* do." Both are `Declared`: they have authority but no automatic evidence. They are written by humans (or proposed by agents and accepted by humans) and exist independently of the code that realises them. The graph distinguishes — sharply — between a requirement and the test that witnesses it: the requirement is intent; the test is observed evidence; the realisation is a `CodeArtifact` linked through `se:realizes`.

This three-way distinction (intent / realisation / witness) is the structural payoff of using a typed knowledge graph for this purpose. A grep cannot tell you that a requirement has no test asserting it. An EigenQL query can.

## 4. Ingestion architecture

### 4.1 Source remains canonical (with a stated exception)

For the bidirectional-index portion of the codebase, the git repository is the source of truth. The graph is *derived* from it. The trust direction is: source files → ingestion Components → typed Resources. If the graph and the source disagree, the source wins; the graph gets re-derived.

This is a strict commitment, with one stated exception: the selective model-driven slice in §6 inverts the trust direction for a deliberately small set of schema-shaped artifacts (boundary contracts, the ESL grammar, the EigenQL surface, the gRPC API, the Eigon serialisation shape, and out-of-core layer storage keys). For those artifacts and only those, the graph is canonical and the source-tree files are regenerated outputs whose drift is an error. §6 develops this and is the only place §4.1's source-wins rule is suspended.

For everything else, the source-first stance holds. Agent edits flow into source files first (via the ordinary file-editing tools); the graph is then refreshed from the resulting tree. The graph cannot hold information that does not exist in some form in the source — anything purely-graph-side (the `se:realizes` link, the `se:Intent` resource) lives in a small set of *graph-side* files committed alongside the code (a `.eigenius/` directory of ESL declarations, treated like build metadata).

### 4.2 The ingestion Components

Each language gets a single ingestion Component, registered with `capability_level: Read` (it reads the filesystem; it does not mutate the world). A note on terminology: `capability_level` is the per-Component effect tag from D3 §6.3 with values `Pure | Read | IO`; this is distinct from the four-tier *evaluator capability mode* (`Pure | Read | Check | IO`) discussed in `guides/esl/08-capability-modes.md`, which classifies the kernel's evaluator behaviour rather than the Components it dispatches to.

```
se:ingest_rust         : se:RustSource     → core:resource_array
se:ingest_typescript   : se:TsSource       → core:resource_array
se:ingest_julia        : se:JuliaSource    → core:resource_array
se:ingest_python       : se:PythonSource   → core:resource_array
se:ingest_shell        : se:ShellSource    → core:resource_array
se:ingest_bicep        : se:BicepSource    → core:resource_array
se:ingest_markdown     : se:MarkdownSource → core:resource_array
```

Each takes a tree of source files and returns the typed `CodeArtifact` / `Module` / `TestCase` / `Doc` Resources extracted from them. The implementations vary:

- **Rust** uses `syn` for parsing and `cargo metadata` for the crate graph; the largest and most-load-bearing extractor.
- **TypeScript** uses the TS Compiler API (the same one `tsc` and `tsserver` use) for AST and symbol information.
- **Julia** uses the Julia parser exposed via the existing D27 Julia institutions infrastructure — re-using machinery that already exists in the platform.
- **Python** uses `ast` plus `inspect` for runtime types where the codebase exposes them.
- **Shell** uses `shellcheck`'s parser, lifted out of its lint role.
- **Bicep** uses the `bicep build --json` output as the structured source.
- **Markdown** uses a frontmatter-aware parser, with special-case handling for the `design/` and `guides/` directory conventions to attach `se:Doc` Resources to the right `DesignElement`s and `Module`s.

Every ingester is a normal Component; none requires kernel changes; all are content-addressed for cache hits. Per D6's trace memoisation, a Component invocation is keyed by `hash(component_iri, canonicalize(input), canonicalize(argument))` — re-ingesting an unchanged file is free; re-ingesting after a one-line change touches only the artifacts whose canonicalised input shape changed.

### 4.3 One layer per commit

The commit-to-layer mapping is the natural unit for incremental graph updates, given Eigenius's immutable-layer model. A new commit on `main` produces a new Layer in a dedicated context (e.g. `urn:eigenius:context:repo:eigenius/main`). Branch heads are tracked layer chains; merging a branch produces a layer that sits on top of both predecessors (a non-linear chain — the existing layer machinery already supports this).

For agent work-in-progress, a related pattern is appropriate: an agent operating against a not-yet-committed working tree builds an ephemeral layer that derives off the latest committed layer and never merges into the chain unless and until the change is committed. This generalises the FIBER-without-INTO pattern from D14 §9.4 (in which a FIBER query result is materialised inside the query's evaluation but, absent an `INTO` clause, never enters the regular chain). The same shape applies here: agent-side analyses produce typed Resources, those Resources are queryable through the normal EigenQL surface, and they vanish from the chain history if the underlying change is discarded.

A coarser commit rate may be appropriate at scale (one layer per merged PR, one layer per N commits, one layer per release). The mechanism does not change; only the cadence does. This is a tunable, not a structural decision.

### 4.4 Cross-language symbol resolution

Most of the cross-language work in the Eigenius codebase is at well-defined boundaries — the gRPC API between kernel and orchestrator, the WASM ABI between kernel and institutions, the JSON shape between orchestrator and notebook. Those boundaries are precisely where boundary contracts already live (or should). Cross-language `se:depends_on` edges therefore route through `contracts:BoundaryContract` as the intermediate Resource: a TypeScript caller depends on a Rust function *via* the contract that both sides implement. The graph models this directly (`TS function → contracts:OperationContract ← Rust function`), which is both more accurate (neither side directly depends on the other; both depend on the contract) and more useful (changing the contract ripples to both sides via a single query).

## 5. Validation institutions

The reactive-validation pattern from D14 — `QueryClass(AutoOnLoad)` returning a `Verdict` of `Holds | Fails | Undecidable`, gating the Load operation — is the obvious fit for code validation. Three institutions cover the bulk of the work; a fourth handles formal verification for the slices that warrant it.

### 5.1 The `Lint` institution

A single institution registering a per-language QueryClass. When a `RustModule` Resource enters the chain, the `lint:Rust` QueryClass fires, runs `cargo clippy` against the corresponding source path, and returns a Verdict. `Holds` means clean; `Fails` carries the diagnostics as structured `se:LintFinding` resources attached to the failing Module; `Undecidable` covers parse failure or toolchain error.

By riding `AutoOnLoad`, lint failures *gate the Load operation* (D14 §4.4). A code change cannot enter the layer chain in a state that fails lint. This is much stronger than CI-after-the-fact: the graph cannot reach a state in which a `Holds` lint Verdict is attached to code that does not lint.

### 5.2 The `TypeCheck` institution

The same pattern, lifted to compiler invocation. `cargo check` for Rust, `tsc --noEmit` for TypeScript, Julia's type inference, `mypy` for Python (where annotations exist), `bicep build --no-restore` for Bicep validation. Shell is excluded — it has no static type system worth wrapping — but `shellcheck` results route through the Lint institution.

The TypeCheck Verdict shape is richer than Lint's: `Fails` carries `se:TypeError` resources whose properties include the affected `CodeArtifact`, the expected and actual types, and the source range. This is the structured form an agent would otherwise extract from `cargo check --message-format=json`, materialised as queryable Resources.

### 5.3 The `TestRunner` institution

Test execution is more delicate because tests are slow, sometimes flaky, and not always deterministic. The institution's QueryClass uses dispatch role `OnDemand` rather than `AutoOnLoad`: tests run when explicitly queried (typically by an agent asking "do the tests for the module I just changed pass?"), not on every Load.

The institution exposes operations:
- `runner:RunSuite { crate: ?c }` — runs the suite, returns `se:TestRunResult` resources.
- `runner:RunImpacted { changeset: ?cs }` — runs only tests whose `se:covers` edges intersect the artifacts changed by the ChangeSet. This is the test-impact-analysis loop that AI coding agents need; it's a normal EigenQL query against the graph plus a Component invocation, not a separate machinery.
- `runner:Coverage { suite: ?s }` — runs the suite under instrumentation, materialises the resulting `se:covers` edges back into the graph.

Trace memoisation makes this naturally incremental: re-running the suite after an unchanged commit is a cache hit; re-running it after a one-file change re-runs only the impacted tests *and* re-materialises only the coverage edges that touch the changed artifacts.

### 5.4 The `ProofCheck` institution

For the Verus-annotated portions of the Rust kernel and any Lean 4 specifications introduced via D28, a ProofCheck institution dispatches the proof to the relevant checker and, on success, materialises an `se:VerifiedProperty` resource carrying the proof term. This is the Verified rung of the epistemic ladder for code: a `CodeArtifact` linked to a `VerifiedProperty` is one whose claim is mechanically checked.

The QueryClass dispatch role is `AutoOnLoad`, aligning with the worked example in D14 §11.6 — a proof obligation is a sentence in the institution's logic, and the right semantics is for a failed re-check to gate Load on the layer that would have admitted the now-broken proof. Trace memoisation handles the steady-state cost: when neither the proof term nor the artifact has changed (both content-addressed), the re-check is a cache hit, so the `AutoOnLoad` cost is paid only when something actually moved.

### 5.5 What this gives the agent

An agent looking at a function it intends to modify can ask, in a single EigenQL query, for the structural starting context — the function itself, its boundary contract, the tests asserting its behaviour, and any verified properties attached to it:

```eigenql
USING "urn:eigenius:se:RustFunction",
      "urn:eigenius:se:TestCase",
      "urn:eigenius:se:Requirement",
      "urn:eigenius:se:VerifiedProperty",
      "urn:eigenius:contracts:BoundaryContract"

MATCH RustFunction(?f) {
    short_name: "commit",
    declared_in: ?m,
    contracted_by: ?bc
},
BoundaryContract(?bc) {
    declared_operations: ?op
},
TestCase(?t) {
    covers: ?f,
    asserts: ?req
},
Requirement(?req) { priority: ?p }
RETURN [] {
    function: ?f,
    module: ?m,
    contract: ?bc,
    operation: ?op,
    test: ?t,
    requirement: ?req,
    priority: ?p
}
ORDER BY ?p DESC
```

A second, narrower query handles the verified-properties side, which is naturally cardinality-zero-or-many per function and so is awkward to fold into the same `MATCH`:

```eigenql
USING "urn:eigenius:se:RustFunction",
      "urn:eigenius:se:VerifiedProperty"

MATCH RustFunction(?f) { short_name: "commit" },
      VerifiedProperty(?vp) { about: ?f }
RETURN [] { function: ?f, verified_property: ?vp }
```

That is the structural starting context an agent today reconstructs from `grep` and prose-reading every time it touches the code. Here it is two queries, returning typed Resources the agent can act on directly.

## 6. The selective model-driven slice

For a small set of artifacts in the Eigenius codebase the source-of-truth direction inverts: the graph is canonical, code is generated. The criterion for inclusion is straightforward — these are artifacts whose *natural form* is already a structured specification, not handwritten code, and where regeneration is cheap and uncontroversial.

The proposed initial slice:

| Artifact | Source-of-truth in graph | Generates |
|---|---|---|
| Boundary contracts | `contracts:BoundaryContract` resources authored in ESL | Rust trait skeletons + TS interface declarations + JSON schema for the contracted operations |
| ESL grammar | The grammar itself as Eigon resources (a meta-grammar institution) | The `pest` / `lalrpop` source for the parser |
| EigenQL surface syntax | Same — grammar as resources | Parser source |
| gRPC API | `BoundaryContract` for the kernel-orchestrator interface | The `.proto` file and the generated `tonic` server skeleton |
| Eigon serialisation shape | `Class` and `Property` declarations for the wire format | The Rust `serde` derives and the TS type definitions |
| Out-of-core layer storage keys | The encoding rules from D4 expressed as a resource | The Rust key-encoding module |

The mechanism for each is the same. A codegen Component reads the relevant graph subgraph, produces the source artifact, and writes it to a designated path under the source tree. A *drift detector* runs as a `QueryClass(AutoOnLoad)` on the source-tree-side: if the file on disk differs from the regenerated output, the Verdict is `Fails` and the Load is rejected. This makes "graph and code disagree about the contract" a structural impossibility for these slices, not a runtime drift to chase down later. As noted in §4.1, this is the one place the source-wins rule is suspended in favour of graph-wins.

The slice is deliberately small. Expanding it is a per-artifact decision, not a programme. The criterion for adding an artifact is: (a) the artifact is fundamentally a specification rather than handwritten logic; (b) the regeneration is mechanical; (c) the human-edit-the-generated-file failure mode is acceptable (caught by the drift detector but not silently overwritten). A handwritten algorithm fails (a). A complex piece of business logic with many tasteful local decisions fails (b). Boundary contracts pass all three; algorithm internals fail (a) and (b).

This selective MDD approach gives Eigenius the model-driven properties precisely where they earn their keep — at API surfaces and protocol boundaries, where the cost of source-and-spec drift is highest — without imposing model-driven discipline on the rest of the codebase.

## 7. Agent integration

Coding agents interact with the SE knowledge graph through the existing TypeScript SDK described in D22 — the `Eigen` facade over the Connect-RPC orchestrator, never directly against the kernel's gRPC. This is the integration surface; it does not need to be rebuilt.

### 7.1 Read patterns

Most agent interactions are read-heavy. The SDK exposes the EigenQL query surface; the agent sends queries shaped by the task at hand. Representative shapes:

- *Localisation:* "what `CodeArtifact`s realise the `DesignElement` named `D6.executor.dispatch`?"
- *Discovery:* "find `CodeArtifact`s and `Doc`s whose descriptions match — lexically or semantically — the natural-language description of the task the agent is starting." (See §7.4: this pattern depends on EigenQL gaining text and vector retrieval as built-in primitives.)
- *Coverage:* "for this `Requirement`, list the `TestCase`s with `se:asserts` edges to it; for any with no test, mark them as test-gaps."
- *Impact:* "if the `BoundaryContract` for `kernel.Load` changes, which `CodeArtifact`s on either side need revisiting?"
- *History:* "for this function, what `ChangeSet`s touched it in the last N layers, and which `se:Author`s authored them?"
- *Justification:* "for this `AnalysisResult`, traverse the reasoning trace back to the `Component` invocations that produced it and the `Resource`s they consumed."

All are EigenQL queries. The structural patterns (Localisation, Coverage, Impact, History, Justification) need no platform extensions; Discovery is the one that does, and §7.4 develops what it requires.

### 7.2 Write patterns

When an agent proposes a change, the write pattern is:

1. **Record intent.** The agent emits an `se:Intent` resource: a Declared statement of what it plans to do, against which Requirements, with what expected effect on which CodeArtifacts. This is the agent's reasoning made queryable — not an after-the-fact log, but a first-class resource that exists *before* the change.
2. **Make the change in source.** Through the ordinary file-editing tools. The graph is not yet aware.
3. **Re-ingest.** The ingestion Components run against the modified tree, producing new versions of the affected CodeArtifact resources. The trace cache means only changed-or-dependent files are re-extracted.
4. **Validation institutions fire on Load.** Lint and TypeCheck run automatically; the change fails to enter the layer chain if either reports `Fails`. This is the agent equivalent of "must compile before commit" — but enforced as an invariant of the graph state, not a CI step.
5. **Assert the structural links.** The agent writes the `se:realizes`, `se:contracted_by`, and (if it added or modified tests) `se:asserts` edges that the structural extractors cannot infer. These edges are the agent's *claim* about what the change does; they are Declared resources alongside the Observed code.
6. **Trigger relevant tests.** `runner:RunImpacted` against the resulting ChangeSet runs only the tests whose coverage intersects.
7. **Promote intent → outcome.** The `se:Intent.results_in` edge points to the resulting `ChangeSet`, closing the loop: future queries can ask "what intents did this agent record over the last week, and which of them produced merged ChangeSets that pass their associated tests?"

The structure is deliberately uneventful. No new mechanism is invoked; everything rides on Resources, Components, the existing institution surface, and the existing layer model.

### 7.3 Reasoning traces as agent memory

The reasoning-trace machinery — every Component invocation is a typed Resource recording inputs, outputs, intermediate values, latency, and (for LLM Components) prompt and response — applies unchanged. For an agent, the reasoning trace is *its memory*. A subsequent agent task can query "for the previous `Intent` against this Requirement, what alternatives did the agent consider before settling on this approach?" — and get a structured answer derived from the actual trace, not a summary the previous agent had to remember to leave behind.

This is the most distinctive thing the platform offers a coding agent: it makes the agent's history queryable in the same language and against the same store as the codebase the agent is working on.

### 7.4 Text and vector retrieval as EigenQL primitives

The §7.1 read patterns are mostly structural; the Discovery pattern is not. Many real agent queries begin with a fuzzy concept rather than a known IRI — "find code related to WAL truncation," "find requirements semantically similar to this proposed change description," "find docs discussing the layer-merge invariant." These are retrieval problems against the natural-language content of Resources (descriptions, doc bodies, comments, requirement statements, commit messages) and against embeddings derived from that content.

This proposal depends on EigenQL gaining text and vector retrieval as **built-in primitives**, not as separate institutions invoked through `FIBER`. The rationale is operational rather than philosophical: every read pattern an agent issues benefits from being able to mix structural pattern matching with full-text and semantic search in a single query, and the planner needs to know about indexes to push selective predicates down and reorder joins. Treating retrieval as an institution would force every hybrid query across `FIBER` boundaries, defeat planner-level pushdown, and impose awkward score-as-attribute marshalling on the consumer.

The minimum surface required — as it landed in D43 v1 after the June 2026 surface review:

- **Field-level text indexing**, marked by an ESL `text_index` declaration on a `Property` (or, equivalently, a `core:TextIndex` Resource targeting the Property). The kernel maintains a layer-aware inverted index per indexed property — additions in a new layer become queryable as of that layer; the index is queryable at any historical layer.
- **Field-level embedding**, marked by an ESL `vector_index` declaration naming the embedder Component IRI. The kernel runs a post-Load sweep against the active Embedder; embeddings are content-addressed by `(source_content_hash, model_iri)` so unchanged input produces cache hits.
- **A single `~` similarity operator** between a property-bound variable and a string literal (or, with the optional trailing `{ via:, model:, k:, limit: }` hint block, an explicit override). The platform picks the strategy from the active index set: text-only if only a TextIndex is active, vector-only if only a VectorIndex is active, hybrid (RRF-fused internally) if both. The user never names the embedder, never writes `EMBED(...)`, never names a fusion function. See [D43 §3.3 / §3.4](d43-text-and-vector-retrieval.md) for the full surface and the rationale for collapsing the original D35 draft (`TEXT_MATCH` / `VECTOR_NEAR` / `EMBED` / `RRF` / explicit scores) into one operator.
- **`TOP N`** as the ranked-truncation surface. When `WHERE` contains `~` operators, `TOP N` orders the result by the platform-internal fused similarity score and truncates. `LIMIT N` stays the un-ranked surface for structural-only queries.

A representative hybrid query that an agent might issue when starting work on a previously-unfamiliar area:

```eigenql
USING "urn:eigenius:se:CodeArtifact",
      "urn:eigenius:contracts:BoundaryContract"

MATCH CodeArtifact(?a) {
    description: ?desc,
    contracted_by: ?bc
}
WHERE ?desc ~ "WAL truncation concurrent commit"
   OR ?desc ~ "rolling back a partially-written commit under concurrent load"
RETURN [] {
    artifact: ?a,
    contract: ?bc
}
TOP 20
```

The collapsed shape preserves the original query's intent — find code artifacts whose description is related to either of two phrases, joined with their boundary contract, ranked by relevance, top 20 — while eliminating five surface concepts the user never needed (the embedding vector, two explicit score functions, an explicit fusion function, and the `BY <expr>` ranking key). The platform composes text + vector probes internally, fuses via RRF (k=60 default; overridable per operator via `{ k: ... }`), and feeds the fused score into `TOP`'s ranking.

An end-to-end integration test pinning this shape lives in [`kernel/tests/d35_se_retrieval_worked_example.rs`](../../kernel/tests/d35_se_retrieval_worked_example.rs) (D43 M9.1).

Two operational points worth flagging for the EigenQL implementers:

- **Layer-aware HNSW is the hard part.** Tantivy's segment model is naturally layer-friendly — each new layer adds segments; queries union over the segments visible at the queried layer. HNSW is harder; the obvious approach is per-layer-segment HNSWs with merge-on-compact, paying a query-time fanout cost in exchange for layer correctness. Worth prototyping early.
- **Embedding generation is `NonDeterministic` across model versions but stable across calls.** Embeddings are content-addressed by `(source_content_hash, model_iri)`, so unchanged input produces cache hits — but agent queries should pin the embedding model IRI explicitly when correctness matters across model upgrades (via the operator's `{ model: ... }` hint), falling back to the active VectorIndex's declared model otherwise.

This is properly D2's territory rather than D35's. What D35 commits to is that the SE knowledge graph cannot deliver the agent read patterns of §7.1 without these primitives in place. Treat it as a hard dependency satisfied by D43 v1.

## 8. Phased rollout

The proposal is bootstrapping-friendly. Each phase delivers value on its own; later phases extend rather than replace earlier ones.

| Phase | Deliverable | Risk |
|---|---|---|
| 0 | `urn:eigenius:se` ontology declared in ESL, loaded into a fresh context. No ingesters yet. | Low — pure schema work. |
| 1 | Rust ingester (`se:ingest_rust`). Manual one-shot ingestion of the Eigenius repo at a pinned commit. | Medium — `syn` plus `cargo metadata` is well-trodden, but the artifact-decomposition design needs care. |
| 2 | `Lint` and `TypeCheck` institutions for Rust. AutoOnLoad gating works against the layer chain. | Medium — institution registration is well-defined; the toolchain wrappers are standard. |
| 3 | `TestRunner` institution for Rust with `RunImpacted` and `Coverage`. The first end-to-end demonstration of test-impact querying against the graph. | Medium — coverage instrumentation has known sharp edges in Rust. |
| 4 | TypeScript ingester + TypeCheck institution. The orchestrator and notebook code enter the graph. | Low — TS Compiler API is a stable substrate. |
| 5 | Markdown ingester for `design/` and `guides/`. The existing documentation corpus becomes queryable; `se:documented_by` edges are populated from frontmatter and naming conventions. | Low — heuristic-driven but tolerant. |
| 6 | First selective-MDD slice: regenerate the Rust `BoundaryContract` trait skeletons from `contracts:BoundaryContract` resources, with the drift detector as a QueryClass(AutoOnLoad). | High — first inversion of trust direction; needs careful UX for the "edit the spec, not the generated file" workflow. |
| 7 | Agent integration: a small TypeScript SDK extension exposing the read/write patterns of §7 as named operations. The notebook gains a "show me the structural context for this artifact" panel. | Medium — depends on §7.2's write loop being uneventful in practice. |
| 8 | Julia, Python, Shell, Bicep ingesters. The remaining 5% of the codebase enters the graph. | Low — no new mechanisms, just per-language extractor work. |
| 9 | `ProofCheck` institution wired to Verus and (per D28) Lean 4. `VerifiedProperty` resources start appearing for the most safety-critical kernel paths. | Medium — depends on D28 maturity and the existing Verus integration. |

Phases 0–3 are the minimum viable demonstration: an agent can localise, query coverage, and validate changes against the Rust kernel through the graph. Phases 4–7 expand to the full repo and close the agent loop. Phases 8–9 round out the polyglot story and the verified-knowledge slice.

## 9. Open questions and risks

**Layer cadence.** One layer per git commit may be excessive in practice — the Eigenius repo's commit rate is high, and a layer carries fixed overhead. A coarser cadence (per-PR, per-merge-to-main, daily) preserves the model without the storage cost. The decision can be deferred but should be made before phase 3.

**Source vs. graph authority for non-source artifacts.** `se:realizes`, `se:Intent`, `se:satisfies` are claims that don't appear in the source files. Where do they live? The proposal's tentative answer — a `.eigenius/` directory of ESL declarations committed alongside the code — works but adds a second authoring surface. An alternative is a graph-side store outside the repo, with the repo holding only a hash of the corresponding graph state. The trade-off is between offline-friendliness (the `.eigenius/` directory lives in git, works without the kernel) and freedom from human-edit-the-ESL-by-hand pain (the graph-side store is authoritative, ESL is generated).

**Resource granularity for very large artifacts.** A single `RustFunction` Resource is fine. A `RustCrate` with thousands of artifacts is fine — they're separate Resources. But a single 500-line function with deep cyclomatic complexity is borderline: do we represent its internal control flow as Resources, or treat the whole function as the atomic unit? The proposal treats functions as atomic; deeper structure is left to extractor Components invoked on demand. Revisit if practice shows this is wrong.

**Out-of-core for the full repo.** The Eigenius codebase plus full ChangeSet history plus all coverage edges plus all reasoning traces will exceed in-memory comfort. D23's out-of-core layer architecture is the obvious answer; this proposal does not introduce new requirements on it but does push it into the critical path.

**Schema evolution of `urn:eigenius:se` itself.** This is *not* a D24 question (D24 is the on-disk RocksDB shape). It is a layer-and-`lifecycle_policy` question: the SE ontology should ride the same evolution mechanism as any other ontology — new layers, additive changes preferred, breaking changes signalled by `lifecycle_policy: Breaking` on the affected Class declarations. The proposal expects to make several breaking changes during phases 0–3 and stabilise from phase 4 onward.

**LLM Component non-determinism.** An LLM-backed code-generation Component is `NonDeterministic` and its output is not naturally cache-keyed. The platform's existing answer — `Result<A, E>` for first-class failure handling, explicit determinism tagging on Component declarations — applies, but the operational consequence is that LLM Component traces are *records of what happened*, not *cache hits to reuse*. This is correct, and the proposal does not try to subvert it.

**Selective MDD's UX.** Phase 6 is the most novel piece. The author-the-spec workflow for boundary contracts is qualitatively different from edit-the-source. If contributors find it onerous — if the round trip from ESL edit → regenerate → review the generated trait → run tests is too slow or too opaque — adoption will stall. The notebook surface (D22) needs first-class affordances for this workflow, not an after-the-fact CLI dance.

**Trust in the graph for safety-critical decisions.** A coding agent that trusts the graph's `se:covers` edges to decide which tests to run is, in effect, trusting the coverage Component's accuracy. If the Component is wrong, tests are skipped that should have run. The epistemic categories help here — a `Derived` `se:covers` edge is not the same epistemic claim as a `Verified` one — but the agent's tooling needs to reason about *which* derivations it is willing to act on without re-confirmation. This is a policy question the proposal raises but does not settle.

## 10. Non-goals

To be explicit about what this proposal does *not* claim:

- **A workflow engine.** Programs are dependent functions (D3); they are not DAGs of tasks. An agent's plan is a Program whose Components are tools, not a graph of stages. Anyone reaching for `se:Step` or `se:Workflow` is reaching for the wrong abstraction.
- **OWL-style reasoning.** Architecture-v0.3 §3.9 (the Decidability Boundary) excludes transitive properties and property chains from the schema; OWL union/intersection are admissible only as registered capabilities, not as built-in inference. Closures are EigenQL recursive rules; the proposal does not propose reintroducing them as schema.
- **Comorphism composition.** A comorphism in D14 is a triple $(s, m, t)$ — ExportFormat, EigenTT term, ImportFormat — declared between two institutions (D14 §5). The kernel deliberately does not close the comorphism set under composition (D14 §5.2; the underlying reason is Diaconescu's Fact 14.9 — composing left adjoints yields only an isomorphism, not equality). Cross-fibre translations the SE story needs (e.g., "a Rust `BoundaryContract` ↔ a TS interface") are declared as direct comorphisms, not synthesised.
- **A kernel rewrite.** The proposal lives entirely in ontology, ingestion Components, validation institutions, and SDK extensions. The kernel's four operations (Load, Query, Validate, Reflect) and the Component / Institution / Layer mechanics are unchanged.
- **Replacing the existing development workflow.** Cargo, `tsc`, the Julia REPL, `pip`, the existing CI — all continue to work. The graph is an additional substrate, not a replacement.
- **Universal MDD.** §6 is small on purpose. The hybrid approach is the design, not a transition state on the way to full model-driven development.

## 11. Relationship to other design documents

This proposal sits as a peer of the existing D-series; it does not modify any of them. The principal dependencies:

- **[D2 EigenQL specification](d2-eigenql-specification.md):** the Discovery read pattern of §7.1 depends on EigenQL gaining text and vector retrieval as built-in primitives. §7.4 sketches the minimum surface (field-level `text_index` and `embed` annotations on Property declarations; `TEXT_MATCH` / `TEXT_SCORE` / `VECTOR_NEAR` / `VECTOR_SIM` / `EMBED` / `RRF`; `TOP K BY ?score`); the actual language specification belongs in D2's next iteration. This is a hard dependency, not an optional enhancement.
- **[D3 program model](d3-program-model.md):** agent operations are Programs; the dependent-function shape and EigenTT term grammar are non-negotiable. `capability_level` on Components is sourced from D3 §6.3.
- **[D14 institution realisation](d14-institution-realisation.md):** the validation institutions (Lint, TypeCheck, TestRunner, ProofCheck) ride D14's three-method trait and the QueryClass dispatch-role mechanism. `AutoOnLoad` is what makes graph state coherent with code state; `OnDemand` is the right fit for tests; the ProofCheck dispatch role aligns with D14 §11.6's worked example.
- **[boundary contracts](boundary-contracts.md):** the SE ontology *bridges into* boundary contracts via `se:contracted_by`. The contract vocabulary — `BoundaryContract.declared_operations`, `OperationContract`'s effects/determinism/idempotence/error/preconditions/postconditions/resource_bounds, the formalisation spectrum (Documentation/Schema/Runtime/TypeLevel/ProofLevel), and the `ImplementationManifest` with `Trusted | Advisory | NonTrusted` tags — is reused without modification. The proposal pins the namespace IRI as `urn:eigenius:contracts`.
- **[D6 execution architecture](d6-execution-architecture.md):** trace memoisation makes incremental ingestion and incremental analysis automatic. The proposal exploits this throughout.
- **[D22 notebook and TypeScript SDK](d22-notebook-and-typescript-sdk.md):** the agent-integration surface is the existing `Eigen` SDK and the notebook UX. No new SDK is proposed; the existing one gains thin per-domain wrappers.
- **[D23 out-of-core layer architecture](d23-out-of-core-layer-architecture.md):** the proposal pushes scale onto D23 but does not impose new requirements.
- **[D24 schema versioning](d24-schema-versioning.md):** *not* relevant to the SE ontology. D24 is RocksDB shape; SE-ontology evolution rides the layer + `lifecycle_policy` mechanism.
- **[D27 Julia institutions](d27-julia-institutions.md), [D28 Lean 4 as institution](d28-lean-4-as-institution.md):** the Julia ingester reuses D27's parser embedding; the ProofCheck institution dispatches to D28 for Lean 4 verification when present.

---

*This is a draft proposal. The structural commitments — using existing institution and Component mechanisms, modelling code-as-claims rather than code-as-bytes, hybridising bidirectional indexing with selective MDD at well-defined boundaries — are the load-bearing design decisions and should be the focus of review. The phased rollout, the specific class decomposition, and the open questions in §9 are open to revision.*
