# 8. Worked demos

Five end-to-end demos. Three live in [`demo/`](../../../demo/), each driven by a shell script; the multi-institution Julia stack and the Lean verification audit chain live under [`notebooks/examples/`](../../../notebooks/examples/). They're the fastest way to see the platform working as a whole — and the most reliable smoke test that an install is correct.

The first four assume the kernel and orchestrator are running — the easiest path: `EIGENIUS_MOCK_LLM=true docker compose up --build -d` (no API key needed) followed by the demo command. `prose-to-formulas` (§8.5) manages its own kernel container.

## 8.1. `demo/run.sh` — the basic document demo

Source: [`demo/run.sh`](../../../demo/run.sh).

```bash
./demo/run.sh                          # default endpoint http://localhost:50051
./demo/run.sh http://localhost:50051   # explicit
```

What it does, step by step:

| # | Step | Verifies |
|---|------|---------|
| 0 | `curl <orchestrator>/health` | Orchestrator reachable |
| 1 | `eigenius load demo/document.json` | Eigon-JSON load path |
| 2 | `eigenius inspect "urn:eigenius:core:Class"` | Bootstrap ontology resolved |
| 3 | `eigenius query 'MATCH "urn:eigenius:core:Class"(?c) ...'` | EigenQL evaluation |
| 4 | `eigenius run demo/summarize-program.json demo/input.json` | Program execution + IO dispatch |
| 5 | `eigenius load demo/document.esl` | ESL compile-and-load path |
| 6 | `eigenius run demo/summarize.esl demo/input.json` | ESL program execution |

Step 4 is the one that exercises the orchestrator (it dispatches `CompleteText`); steps 5 and 6 demonstrate that ESL files are first-class everywhere — `load` and `run` accept `.esl` directly.

The ESL summarization program ([`demo/summarize.esl`](../../../demo/summarize.esl)) is short enough to read in full:

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:demo";

program ex:summarize : ex:Document -> ex:Document {
    let summary : core:string = CompleteText(input);
    Construct ex:Document { ex:text = summary }
}
```

The demo finishes by printing each output for visual inspection. In mock LLM mode, the summary is a placeholder string; with a real API key, it's an actual model completion.

## 8.2. `demo/patent/run.sh` — the patent analysis pipeline

Source: [`demo/patent/run.sh`](../../../demo/patent/run.sh) and [`demo/patent/README.md`](../../../demo/patent/README.md).

A two-step LLM pipeline that exercises both `CompleteJson` (structured extraction) and `CompleteText` (narrative generation):

```bash
./demo/patent/run.sh
```

Steps:

| # | Step | What happens |
|---|------|---|
| 1 | Load [`demo/patent/patent-ontology.esl`](../../../demo/patent/patent-ontology.esl) | Declares `PatentClaim`, `PatentAnalysis`, `PatentBrief` classes |
| 2 | Load [`demo/patent/transformer-patent.json`](../../../demo/patent/transformer-patent.json) | The "Attention Is All You Need" transformer patent text |
| 3 | Run [`demo/patent/analyze-patent.esl`](../../../demo/patent/analyze-patent.esl) | Pipeline: `PatentClaim → CompleteJson → PatentAnalysis → CompleteText → string → Construct → PatentBrief` |

The pipeline:

```esl
program demo:analyze_patent : demo:PatentClaim -> demo:PatentBrief {
    let analysis : demo:PatentAnalysis = CompleteJson(input);
    let summary  : core:string         = CompleteText(analysis);
    Construct demo:PatentBrief {
        demo:summary = summary,
        demo:analysis = analysis
    }
}
```

`CompleteJson` is constrained by the JSON Schema generated from the `PatentAnalysis` class — its output is *guaranteed* to satisfy that class's `requires` properties (when the LLM returns valid JSON). `CompleteText` then takes that structured analysis and produces a plain-language summary, and the final `Construct` packages both into a `PatentBrief`.

This demo demonstrates the central ergonomic claim of the platform: domain-modelled types drive both the structural validation of LLM output (via `CompleteJson`'s schema constraint) and the type-checked composition of pipelines (via the kernel's NbE checker).

The expected output shape — a `PatentBrief` resource with a structured `analysis` field and a free-text `summary` field — is in [`demo/patent/example-output.json`](../../../demo/patent/example-output.json).

## 8.3. `kinase-institutions` — multi-institution Julia stack

Source: [`notebooks/examples/kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh) and [`notebooks/examples/kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json).

The canonical end-to-end demo for the runtime substrate ([chapter 11](11-runtime-substrate.md)). Brings up **five Julia institutions** wrapping `Symbolics.jl`, `IntervalArithmetic.jl`, `Catalyst.jl`, `OrdinaryDiffEq.jl`, and `JuMP+HiGHS`, plus **three cross-institution comorphisms** that compose them via the chain-typed `formulas:FormulaTerm` shared formula language ([formula language guide](../formula/README.md), D32 §6).

```bash
# Cold first run is heavy (~30–60 minutes — five Julia env builds);
# subsequent runs reuse the buildah cache.
EIGENIUS_MOCK_LLM=true docker compose up -d
./notebooks/examples/kinase-institutions-setup.sh

# Then in a browser: http://localhost:8080/notebooks/
# Import notebooks/examples/kinase-institutions.json and Run All.
```

Two storylines exercised end-to-end:

| Storyline | Comorphism | What's verified |
|---|---|---|
| **Forward simulation** (cells 3–6) | `Catalyst → DiffEq` | A reaction network is committed, an `OdeProblem` with FormulaTerm-typed RHS is hand-authored as the "what the comorphism would produce", and an `OdeSolution` claim fires the DiffEq AutoOnLoad gate. The institution re-integrates the RHS via `OrdinaryDiffEq.solve(Tsit5)` and confirms the closed-form final state within tolerance. |
| **Parameter fitting** (cells 7–9) | `Symbolics → JuMP` | A Kᵢ-fit SSE objective is authored as a `SymbolicExpression` carrying a FormulaTerm; wrapped in a `SymbolicsToJuMPInput` composite; the comorphism reifies it as a JuMP `OptimisationProblem`; an `OptimisesTo` claim fires the JuMP-HiGHS AutoOnLoad gate, which re-solves and verifies `Kᵢ* = 2.0`, `SSE* = 0`. The smart-pow walker keeps the QP in `QuadExpr` rather than `NonlinearExpr` territory. |

Both AutoOnLoad gates produce `Holds` Verdicts that commit back to the chain alongside `RuntimeInvocation` audit anchors.

Cells 12–18 close the [D14 §9.3](../../design/d14-institution-realisation.md) chain-reinsertion contract directly through both surfaces:

- **ESL program** (cells 13–15): a wrapper invokes the `symbolics_to_jump` comorphism via the qualified-name function-call form (`comorphisms:symbolics_to_jump(input)`); the produced `OptimisationProblem` lands at a deterministic content-hash IRI `urn:eigenius:comorphism-output:symbolics_to_jump:<hex>`. See [ESL §9.5](../esl/09-institutions.md#95-invoking-comorphisms-from-esl-programs).
- **EigenQL `FIBER ... INTO`** (cells 16–18): the operational backing of the same translation, dispatched interactively via FIBER, with the user pinning the result at a caller-named IRI. See [EigenQL §7.6](../eigenql/07-fiber-clauses.md#76-into--pinning-the-response-iri).

Both paths use the same `commit_with_validation` machinery — comorphism-translated resources, however dispatched, are first-class chain residents.

The per-institution slow-walks under [`platform/julia-institutions/`](julia-institutions/) cover each piece in isolation; the kinase notebook is the one place the whole stack runs together against a single chain.

## 8.4. `lean-verification` — Lean 4 verification audit chain

Source: [`notebooks/examples/lean-verification-setup.sh`](../../../notebooks/examples/lean-verification-setup.sh) and [`notebooks/examples/lean-verification.json`](../../../notebooks/examples/lean-verification.json).

The end-to-end demo for the platform's first verification institution ([chapter 11](11-runtime-substrate.md) + the Lean institution tutorial under [`platform/lean-institution/`](lean-institution/)). Loads a chain layer carrying a [`LeanProofTerm`](../../design/d28-lean-4-as-institution.md#63-the-leanproofterm-resource--verbatim-bytes--chain-mirrored-proposition) backed by a real `lean4export` payload — the proven theorem is `∀ p : EigeniusFFI.Patient, p.weight ≥ 0 → p.weight + 10 ≥ 10`, proved by `omega` against `Float`'s ordering. AutoOnLoad fires the three-part correspondence check at commit time and produces a `Verdict::Holds` resource. The notebook then walks the closed audit chain D28 §5.7 promises.

```bash
EIGENIUS_MOCK_LLM=true docker compose up -d
./notebooks/examples/lean-verification-setup.sh

# Then in a browser: http://localhost:8080/notebooks/
# Import notebooks/examples/lean-verification.json and Run All.
```

The five resources the setup loads — Patient class, Patient instance, `LeanPackageMirror` (audit anchor with embedded Lake project archive + content-addressed hash), `LeanProofPayload` (the `lean4export` bytes), `LeanProofTerm` (proposition + cross-references) — plus the AutoOnLoad-generated `Verdict` form the closed cycle the notebook walks:

```text
Patient class ← claim instance ← LeanProofTerm → proof bytes
                                         ↓
                                  LeanPackageMirror
                                  ↓             ↓
                            source_layer    mirrored_classes → Patient class
                                                ↑
Verdict (ctor = Holds) ──────── verdict_subject ┘
```

Every byte that went into the verification — Lake project sources, the toolchain pin, the verbatim `lean4export` JSON, the chain-side class declaration, the source layer the mirror anchors to — sits on the chain as a typed, queryable, content-addressed resource. A consumer who wants to reproduce the verdict can pull the archive from `library_content`, fetch the toolchain pinned by `lean-toolchain`, run `lake build && lake exe lean4export`, and re-check the output against the stored bytes.

Verification is **in-process** ([`crates/eigenius-lean/`](../../../crates/eigenius-lean/)) via `nanoda_lib` — no orchestrator round-trip, no IPC, no Docker container spawn. The verdict is a direct function call inside the kernel binary, which keeps the TCB small (D28 §2.3).

Regeneration: when the Lean toolchain or the capstone proof changes, regenerate the fixture with `cargo run -p eigenius-lean --example gen_verification_demo`. Toolchain bumps follow the checklist at [`docs/notes/lean-toolchain-upgrade.md`](../../notes/lean-toolchain-upgrade.md).

## 8.5. `prose-to-formulas` — the same conclusion justified two ways

Source: [`demo/prose-to-formulas/run.sh`](../../../demo/prose-to-formulas/run.sh) and [`demo/prose-to-formulas/README.md`](../../../demo/prose-to-formulas/README.md).

Two sentences of controlled prose from the WRN paper — a measurement (*"MSI cancer models had the exonuclease activity of WRN"*) and an activity claim (*"…required the helicase activity of WRN"*) — go through the DCG parser (D63), which turns each into a closed, felicity-gated `Prop`, committed as an `enc:EncodedClaim` under a `reflection:ProgramTrace` that mints the witness `IsDerivedAs claim_i P_i`. Plus one rule **pinned from the literature**, not from the document: `∀m. HasActivity(m, WRN, exonuclease) → RequiresActivity(m, WRN, helicase)`.

```bash
./demo/prose-to-formulas/run.sh
./demo/prose-to-formulas/run.sh --reparse
```

Under D66 there is **no lift step**: `onco-typed.esl` *defines* the domain predicates over the parser's own lexicon (`def`), so a parsed sentence and its domain formula are the same term by definitional equality. The result is `RequiresActivity(MSI, WRN, helicase)` justified twice — once because sentence 2 asserts it (its own parse witness, nothing Declared), once because it *follows* from sentence 1 plus the published rule specialized at the model with [`spec_poly`](../esl/09-institutions.md#9102-the-justifiedby-certificate-predicate). The derived route carries strictly more assumptions and commits at `Declared`; the point is not that it is better-warranted but that it **knows what it depends on**. Negate the measurement and the two routes come apart in the same run: sentence 2's claim still commits, the derivation that cited sentence 1's parse has nothing left to stand on and is rejected.

Two ways a claim gets justified here, both exercised by [`crates/eigenius-reasoning/tests/justification_routes.rs`](../../../crates/eigenius-reasoning/tests/justification_routes.rs):

| | What warrants it | Grade | Authoring cost |
|---|---|---|---|
| **pinned literature rule** | a published `∀m. A → B` on the chain, specialized with `spec_poly` and applied to a claim an earlier sentence established | Declared | one rule, reused |
| **prose modus ponens** | `A` and `A → B` both parsed from sentences — the grammar renders `if` as native implication | **Derived** | none |

The second is the only one that Declares nothing: `"S₁ if S₂"` parses to a genuine top-level implication whose antecedent is verbatim the premise sentence's own parse, so `app` composes them with no human assertion in between. (A third way — generated **shape rules**, one Declared rule per parse shape — was retired by D66's definitional lift.)

**Prerequisite: an aligned lexicon snapshot** (`…/db-snapshot/wordnet-umls-aligned-d66`, ~993 MB). The propositions are built from lexicon axioms (`wn:v02627934_t` is the verb sense of *require*), so the chain must be the one that *defines* those axioms; a bare core+domain chain fails at the D47 decode with `ConstRef references unresolved IRI`. And it must be the **aligned** chain — on a raw reseed, duplicate WordNet/UMLS senses make `--reparse` fail closed. `run.sh` stages the snapshot into the kernel's docker volume read-only. Override the location with `EIGENIUS_DB_SNAPSHOT`; build one with [`scripts/reseed-lexicon-db.sh`](../../../scripts/reseed-lexicon-db.sh) then `scripts/build-alignment-snapshot.sh`.

## 8.6. Running the demos as smoke tests

Each demo exits 0 on success and non-zero on any step failure. They're suitable as part of CI or pre-deployment verification:

```bash
# Bring up stack, run the demos, tear down
EIGENIUS_MOCK_LLM=true docker compose up --build -d
./demo/run.sh
./demo/patent/run.sh
docker compose down
```

The demos exercise overlapping but distinct subsystems:

| Demo | Exercises |
|---|---|
| `demo/run.sh` | Bootstrap, JSON+ESL load, query, program run with `CompleteText` |
| `demo/patent/run.sh` | `CompleteJson` structured extraction, two-step LLM pipeline, `Construct` |
| `demo/prose-to-formulas/run.sh` | DCG parse → `EncodedClaim`, the definitional lift (`def`, D66), a `spec_poly`-specialized literature rule, prose modus ponens, certificate rejection on edited prose |

For coverage, run both LLM demos. For speed, `demo/run.sh` alone covers the most common failure modes. The prose-to-formulas demo needs the lexicon snapshot staged first (§8.5), so it doesn't belong in a cold CI job.

## 8.7. Customising the demos

Each demo script accepts the kernel endpoint as the first positional argument, so you can point them at a kernel running anywhere:

```bash
./demo/run.sh http://kernel.internal:50051
./demo/patent/run.sh http://kernel.internal:50051
```

For local development variants — different ontologies, different programs — the simplest pattern is to copy the script and modify the file paths.

---

Next: **[9. Building WASM components →](09-wasm-components.md)** (historical — the feature was removed)
