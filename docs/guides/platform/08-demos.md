# 8. Worked demos

Three end-to-end demos in [`demo/`](../../../demo/), each driven by a shell script. They're the fastest way to see the platform working as a whole — and the most reliable smoke test that an install is correct.

All three assume the kernel and orchestrator are running. The easiest path: `EIGENIUS_MOCK_LLM=true docker compose up --build -d` (no API key needed) followed by the demo command.

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

## 8.3. `demo/wasm/run.sh` — WASM extensibility

Source: [`demo/wasm/run.sh`](../../../demo/wasm/run.sh).

Exercises the WASM hosting path end-to-end with two WASM components (one Pure, one IO):

```bash
./demo/wasm/run.sh
```

Steps:

| # | Step | What happens |
|---|------|---|
| pre | Build any missing WASM binaries | Runs `cargo component build` if the `.wasm` outputs aren't present |
| 0 | Health-check the orchestrator | Same as the basic demo |
| 1 | Install [`wasm-doc-validator`](../../../examples/wasm-doc-validator/) into the kernel | Pure component, registered at `urn:example:components:DocValidator` |
| 2 | Install [`wasm-http-shout`](../../../examples/wasm-http-shout/) into the orchestrator | IO component, dispatches `CompleteText` from inside WASM |
| 3 | `capability test` each capability with sample input | Verifies typed dispatch end-to-end |

This demo is the smoke test for the WASM extension surface for components. After running it, two new capabilities are registered in the live kernel (and orchestrator) — `capability list` shows both, and you can invoke them from your own programs.

For D14 institutions (the dock-assay worked example): see [`kernel/tests/d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs), which exercises the full WASM-institution surface end-to-end via auto-registration from the layer chain. Both WASM extension paths are walked through in [chapter 9](09-wasm-components.md) (components) and [chapter 10](10-wasm-institutions.md) (institutions).

## 8.4. Running the demos as smoke tests

Each demo exits 0 on success and non-zero on any step failure. They're suitable as part of CI or pre-deployment verification:

```bash
# Bring up stack, run all three demos, tear down
EIGENIUS_MOCK_LLM=true docker compose up --build -d
./demo/run.sh
./demo/patent/run.sh
./demo/wasm/run.sh
docker compose down
```

The three demos exercise overlapping but distinct subsystems:

| Demo | Exercises |
|---|---|
| `demo/run.sh` | Bootstrap, JSON+ESL load, query, program run with `CompleteText` |
| `demo/patent/run.sh` | `CompleteJson` structured extraction, two-step LLM pipeline, `Construct` |
| `demo/wasm/run.sh` | WASM component install (pure + IO), institution install, dispatch |

For coverage, run all three. For speed, `demo/run.sh` alone covers the most common failure modes.

## 8.5. Customising the demos

Each demo script accepts the kernel endpoint as the first positional argument, so you can point them at a kernel running anywhere:

```bash
./demo/run.sh http://kernel.internal:50051
./demo/patent/run.sh http://kernel.internal:50051
./demo/wasm/run.sh http://kernel.internal:50051 http://orchestrator.internal:8080
```

For local development variants — different ontologies, different programs — the simplest pattern is to copy the script and modify the file paths.

---

Next: **[9. Building WASM components →](09-wasm-components.md)**
