# 14. Notebook

The Eigenius notebook is the most accessible way to use the platform. It is a single-page React app served by the orchestrator at `/notebooks/`; cells run ESL, EigenQL, TypeScript, and program invocations against the live kernel via Connect-RPC; outputs auto-render as typed inspectors, result tables, layer-stack diagrams, and program-trace trees.

If you have the docker stack up (`docker compose up -d`), you already have the notebook — it is bundled into the orchestrator image at build time and serves alongside the RPC endpoints. Open [http://localhost:8080/notebooks/](http://localhost:8080/notebooks/) in a browser.

![The Eigenius notebook — top of the patent-analysis demo](../assets/eigenius_notebook_ux.png)

This chapter is the operational reference for the notebook UX. The TypeScript SDK that powers it (and that you can use programmatically from outside the notebook) gets its own chapter — see [chapter 15](15-typescript-sdk.md).

## 14.1. What's in a notebook

A notebook is an ordered sequence of typed cells. Five cell types are supported:

| Type | What it does | Run dispatch |
|---|---|---|
| `markdown` | Render-only prose (Github-flavoured Markdown). Click the eye/edit toggle to switch to source view. | None — it just renders. |
| `esl` | Eigenius Surface Language. Compiles + commits a layer on Run; output shows resource count + the new layer ID, with an expandable "View layer stack" accordion. | `eigen.load(source, "application/x-esl")` |
| `eigenql` | EigenQL query against the active layer chain. Output renders in a Fluent `DataGrid` with column types from the synthesized Property metadata. | `eigen.query(source)` |
| `typescript` | Sandboxed TS that runs in the browser with the SDK in scope. The cell's `return` value is auto-rendered (Resource → inspector, ResultSet → table, Topology → layer stack, plain object → JSON tree). | `new Function("eigen", "previousOutputs", source)` |
| `program-run` | Form-based program invocation: program IRI + one or more input IRIs. Single input renders as inspector + trace; multiple inputs render as a results table. | `eigen.runProgramByIri(programIri, inputIri)` per input |

Cells are inserted via the hover-revealed `+` between any two cells (and above first / below last). Per-cell toolbar: type label · `Run` (when runnable) · `↑` / `↓` (move) · `🗑` (delete). Notebook-level toolbar (top): editable title · cell count · `Open…` (file picker) · `Save` (browser download) · `Reset` (clear outputs) · `Publish` (commit notebook to a layer; see §14.5) · `Run all` (top-to-bottom, halts on first error).

Source: [`notebooks/`](../../../notebooks/).

## 14.2. Running the notebook

### Production (docker stack)

```bash
docker compose up -d --build
# open http://localhost:8080/notebooks/
```

The orchestrator image ([`deploy/Dockerfile.orchestration`](../../../deploy/Dockerfile.orchestration)) is multi-stage: stage 1 builds the SPA with `vite build`; stage 2 is the Deno orchestrator runtime with `EIGENIUS_NOTEBOOK_STATIC=/app/notebooks` set so the notebook serves alongside the RPC paths on port 8080. Single origin, no CORS, no separate dev server.

The orchestrator drops the notebook route when `EIGENIUS_NOTEBOOK_STATIC` is unset — useful for headless deployments where the notebook isn't needed.

### Development (live HMR)

```bash
cd notebooks
npm install
npm run dev
# open http://localhost:5173/notebooks/
```

`vite dev` serves the SPA on port 5173 with hot-module reload. Connect-RPC traffic is proxied to the orchestrator on `localhost:8080` ([`vite.config.ts`](../../../notebooks/vite.config.ts)), so you still need a kernel + orchestrator running. The proxy paths are `/eigenius.v1.EigeniusKernel/*` and `/eigenius.v1.NotebookService/*`.

This is the path to use when iterating on the notebook itself.

## 14.3. The patent-analysis demo

On first load the notebook seeds with the patent-analysis demo at [`notebooks/examples/patent-analysis.json`](../../../notebooks/examples/patent-analysis.json). Six cells:

1. **markdown** — what the demo does
2. **esl** — the patent ontology (`PatentClaim`, `PatentAnalysis`, `PatentBrief`) plus the `analyze_patent` program
3. **eigenql** — `MATCH ?r {} WHERE ?r LIKE "urn:eigenius:demo:patent:%"` — list patent-namespace resources after the ESL load
4. **esl** — the transformer-patent input as a `resource patent:US10452978B2 : patent:PatentClaim { … }` declaration
5. **program-run** — invoke `urn:eigenius:demo:patent:analyze_patent` against `urn:eigenius:demo:patent:US10452978B2`
6. **typescript** — `return await eigen.layerTopology({ includeResources: false });` — auto-renders the layer stack

Click **Run all**. The first four cells finish in milliseconds; the program-run cell takes ~10–15 seconds (two LLM calls — `CompleteJson` extracts structured analysis, `CompleteText` writes the plain-language summary), then renders the typed `PatentBrief` output above an interactive trace tree (Program → Let analysis → ComponentTrace CompleteJson, Let summary → ComponentTrace CompleteText, with provider/model/token-count/latency on each component node).

Requires `ANTHROPIC_API_KEY` in the orchestrator's environment. Without it (`EIGENIUS_MOCK_LLM=true`), the LLM components return canned responses but the rest of the flow still works end-to-end.

## 14.4. The notebook file format

Notebooks are versioned JSON with a discriminated cell-type union. Schema in [`notebooks/src/persistence/notebook-format.ts`](../../../notebooks/src/persistence/notebook-format.ts):

```typescript
{
  format_version: 1,
  meta: {
    title?, description?, created?, modified?, eigenius_version?
  },
  cells: [
    // Source-bearing cell:
    { id: "<uuid>", type: "markdown" | "esl" | "eigenql" | "typescript", source: "..." },
    // Program-run cell:
    { id: "<uuid>", type: "program-run", program_iri: "...", input_iris: ["...", ...] }
  ]
}
```

`Save` from the toolbar serialises the current store state to this JSON (with `meta.modified` updated to the save time) and triggers a browser download. `Open…` reads a file via `<input type="file">`, validates the shape, and replaces the store contents. Cell outputs are NOT persisted — they're re-derived by re-running the cells.

## 14.5. Publish to layer

Beyond the on-disk file, a notebook can be published as resources in the kernel's knowledge graph. Click **Publish** in the toolbar; the SDK translates the notebook into a `notebook:Notebook` resource referencing one `notebook:Cell` resource per cell, then loads them into a new layer. The accompanying ontology — [`ontologies/notebook/notebook-ontology.json`](../../../ontologies/notebook/notebook-ontology.json) — is part of the kernel's boot chain (5th layer, after core / program / reflection / institution), so publish succeeds without first registering anything.

IRIs are content-addressed:

- **Cell IRI** = `urn:eigenius:notebook:cell:<sha256>` over the cell's structural form (`{cell_type, source}` for source-bearing cells; `{cell_type, program_iri, input_iris}` for program-run cells)
- **Notebook IRI** = `urn:eigenius:notebook:<sha256>` over `{format_version, title, description, cells:[<cellIri>...]}`. Excluded from the hash on purpose: timestamps and `eigenius_version`, so re-saving identical content yields the same Notebook IRI.

Identical cells across notebooks share a single `Cell` resource — useful for tracking "find every notebook that contains this exact ESL load" and similar queries. The ontology supports queries like `MATCH ?n FROM Notebook WHERE ?n.title LIKE "%patent%"` or `MATCH ?c FROM Cell WHERE ?c.source LIKE "%CompleteJson%"`.

Translator source: [`clients/eigenius-ts/src/notebook.ts`](../../../clients/eigenius-ts/src/notebook.ts).

## 14.6. Auto-rendering of cell outputs

The notebook has type-driven renderers under [`notebooks/src/components/output/`](../../../notebooks/src/components/output/):

- `ResultTable` — Fluent v9 `DataGrid` for `QueryResponse.document` (Eigon-CBOR ResultSet decoding)
- `ResourceInspector` — `@id` + `is_a` tags + sorted property table for any CBOR-encoded resource
- `LayerStackView` — vertical stack of layer boxes (head at top, root at bottom) with per-kind counts, walks `PARENT_LAYER` edges to recover the chain
- `TraceTree` — collapsible tree for `ProgramTrace` resources; flattens right-leaning let-chains into siblings so the visual hierarchy matches dataflow order; surfaces input hashes, provider/model, and per-component latency
- `TypeScriptValueView` — duck-typed dispatcher for TS-cell return values (Resource / ResultSet / RunProgramResponse / LoadResponse / Topology / DOM node / object / primitive)

The ESL-cell load output also has a "View layer stack" accordion that lazy-fetches the topology when expanded and renders it in `LayerStackView`.

## 14.7. Where it lives

```
notebooks/
├── src/
│   ├── App.tsx                  # FluentProvider + EigenProvider + Notebook
│   ├── components/
│   │   ├── Notebook.tsx          # Toolbar + cell list
│   │   ├── Cell.tsx              # Per-cell shell (toolbar + body + output)
│   │   ├── CellInsertGap.tsx     # Hover-revealed "+" between cells
│   │   ├── cells/                # MarkdownCell, ProgramRunCell editor
│   │   ├── editors/              # CodeMirror wrapper + ESL/EigenQL language modes
│   │   └── output/               # The renderers listed in §14.6
│   ├── persistence/notebook-format.ts   # NotebookJson + parseNotebook validator
│   └── runtime/
│       ├── EigenProvider.tsx     # React context for the SDK client
│       ├── notebookStore.ts      # Zustand: cells, meta, run state, run actions
│       ├── resultDocument.ts     # Eigon-CBOR ResultSet decoder
│       └── traceResource.ts      # ProgramTrace decoder
├── examples/
│   └── patent-analysis.json      # Seeded on first load
├── e2e/
│   └── patent-demo.spec.ts       # Playwright golden flow
├── playwright.config.ts
├── vite.config.ts                # /notebooks/ base + Connect-RPC proxy in dev
└── package.json                  # @eigenius/client + Fluent UI v9 + CodeMirror
```

The SDK consumed by the notebook is a `file:` workspace dep on [`clients/eigenius-ts/`](../../../clients/eigenius-ts/) — see [chapter 15](15-typescript-sdk.md).

## 14.8. CI

The Playwright e2e at [`notebooks/e2e/patent-demo.spec.ts`](../../../notebooks/e2e/patent-demo.spec.ts) exercises the LLM-free critical path: open `/notebooks/`, assert the cells render, click `Run all`, assert both ESL load outputs appear, assert the EigenQL grid shows patent-namespace IRIs. Wired up in [`.github/workflows/notebooks-tests.yml`](../../../.github/workflows/notebooks-tests.yml) — brings the docker stack up with `EIGENIUS_MOCK_LLM=true`, runs Playwright, uploads the report on failure.

Run locally:

```bash
cd notebooks
npx playwright install chromium  # one-time
npm run test:e2e
```

The test assumes the orchestrator stack is already up at `http://localhost:8080`.

## 14.9. Design references

- [**D22** — Notebook UX and TypeScript SDK](../../design/d22-notebook-and-typescript-sdk.md) — the spec this guide describes
- [**chapter 15** — TypeScript SDK](15-typescript-sdk.md) — the programmatic API the notebook is built on, also usable from your own code

---

Next: **[15. TypeScript SDK →](15-typescript-sdk.md)**
