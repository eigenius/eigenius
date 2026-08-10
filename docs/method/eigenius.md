---
name: eigenius
description: Drive the Eigenius typed knowledge graph platform — load Eigon-JSON / ESL resources, run EigenQL queries, execute typed programs (incl. LLM-backed via CompleteText / CompleteJson), inspect chain state and provenance, dispatch D14 institutions (Julia / Lean / WASM); and the reasoning/provenance stack (epistemic statuses, witnesses, justification certificates) the `reasoning` skill builds on. TRIGGER when the user mentions Eigenius, the kernel, ESL, EigenQL, Eigon-JSON, FormulaTerm, ontologies, layers, branches, institutions, verdicts, reasoning sentences / witnesses / provenance, or asks to load / query / inspect / run something on the platform. Drive it via the MCP server `eigenius` *or* the `eigenius` CLI.
---

# Eigenius

Eigenius is a typed knowledge graph platform with verifiable reasoning. Everything is a **Resource** identified by an IRI; resources commit into immutable **layers** that form a chain. Branches optional. Four epistemic categories: *declared*, *observed*, *derived*, *verified* (the last via Lean 4 proofs).

Three surface languages:
- **Eigon-JSON** — canonical wire format. Property keys are full IRIs. Reserved key: `@id`.
- **ESL** — human surface that compiles to Eigon-JSON. HCL-style declarations + ML-style expressions.
- **EigenQL** — typed stratified Datalog over the chain (MATCH / DEFINE / RETURN / FIBER).

**Two drivers, one chain.** Operate the platform via the **MCP tools** below *or*
the **`eigenius` CLI** (§CLI surface) — equivalent surfaces; use whichever fits
the context. This skill is the *mechanics*. For the **method** — capturing
reasoning as graded, witnessed propositions the commit gate accepts or rejects —
see the **`reasoning`** skill, which builds on §Reasoning & provenance surface
below.

## MCP tool surface (14 tools)

Use these via `mcp__eigenius__<tool>`.

| Tool | When to use |
|------|-------------|
| `eigenius_health` | Smoke-check before destructive operations |
| `eigenius_query` | Read state via EigenQL — preferred for "show me X" |
| `eigenius_inspect` | Get one resource by IRI |
| `eigenius_load` | Write Eigon-JSON to the chain (commits a new layer) |
| `eigenius_validate_program` | Type-check a program before running |
| `eigenius_run_program` | Execute program + input (both inline JSON) |
| `eigenius_run_program_by_iri` | Execute a program already loaded in the chain |
| `eigenius_list_institutions` | Discover D14 institutions, QueryClasses, comorphisms |
| `eigenius_get_schema` | JSON Schema for a class — feed to LLM JSON-mode |
| `eigenius_list_branches` | Chain navigation: branches |
| `eigenius_list_tags` | Chain navigation: tags |
| `eigenius_list_tasks` | Observe long-running programs (D21) |
| `eigenius_get_task_status` | Detail on one task UUID |
| `eigenius_layer_topology` | Orient — graph of classes / properties / institutions per layer |

## CLI surface (equivalent to MCP; use when in a shell)

The `eigenius` CLI (`target/{debug,release}/eigenius`; build with
`cargo build -p eigenius-cli`) is the same driver from the shell. Pass
`--endpoint http://localhost:50051` to hit the running kernel, `--json` for
machine-readable output. Full reference (every subcommand, flags, exit codes):
**[docs/guides/platform/04-cli-reference.md](https://github.com/eigenius/eigenius/blob/main/docs/guides/platform/04-cli-reference.md)**.

| MCP tool | CLI |
|---|---|
| `eigenius_query` | `eigenius --endpoint … query '<EigenQL>'` |
| `eigenius_inspect` | `eigenius --endpoint … inspect <IRI>` |
| `eigenius_load` | `eigenius --endpoint … load <file.esl\|.json>` (accepts ESL directly) |
| `eigenius_run_program(_by_iri)` | `eigenius --endpoint … run <program> <input>` |
| `eigenius_validate_program` | `eigenius program-validate <file>` |
| `eigenius_list_institutions` | `eigenius --endpoint … list-institutions` |
| `eigenius_get_schema` | `eigenius --endpoint … get-schema <CLASS_IRI>` |
| `eigenius_list_branches` | `eigenius --endpoint … branch list / show / create / delete` |

CLI-only capabilities the `reasoning` protocol uses: **`branch create <task-slug>
--from <head>`** (the per-task branch); **`data attach/verify/validate/provision`**
(D53 external files); **`env build --language r|oci`** + **`run`** (wrapped-R
warrants D55/D56, or *any* pinned tool via the generic `oci` runtime D60 — both
commit a `ProgramTrace → IsDerivedAs` that a `derived(...)` certificate discharges);
**`reflect <trace>`** (record a trace). `validate` / `compile` also run in-process on
local files (no `--endpoint`).

## Vocabulary

- **IRI** — `urn:eigenius:<namespace>:<name>`. Always full in Eigon-JSON; never short.
- **Layer** — immutable commit, content-addressed by SHA-256 over CBOR; identified by `layer_id` (hex).
- **Branch** — named pointer to a layer head. Default: `main`.
- **`is_a`** — every resource declares its classes via `urn:eigenius:core:is_a` (array of class IRIs).
- **Institution** — D14 reasoning system (Julia / Lean / WASM / in-process Rust).
- **InductiveType** — chain-resident inductives EigenTT can `Case`-split on. Distinct from `Class` (the ontology-level shape). A few are dual-declared as both (e.g. `Verdict`) when both shapes are needed. Currently loaded: `core:Option`, `formulas:FormulaTerm`, `lean:LeanExpr` / `LeanLevel` / `LeanLevelList` / `LeanName`, `institution:Verdict`.
- **`FormulaTerm`** — `urn:eigenius:formulas:FormulaTerm`. Chain-mirrored EigenTT fragment shared by every numerical institution.
- **Verdict** — `Holds` / `Fails` / `Undecidable`. The chain-resident outcome of an AutoOnLoad or Decidable QueryClass dispatch.
- **AutoOnLoad** — institution gate fired automatically on commit when a resource of the bound class enters the chain.
- **FIBER** — EigenQL clause that dispatches to an institution. `FIBER … AS ?var INTO "<iri>"` commits the response as a chain-resident resource.

## Minimal shapes

Eigon-JSON resource (loaded via `eigenius_load`):
```json
{
  "@id": "urn:eigenius:demo:rex",
  "urn:eigenius:core:is_a": ["urn:eigenius:demo:Dog"],
  "urn:eigenius:demo:name": "Rex"
}
```

ESL (compiles to the above; the kernel accepts `.esl` directly via `content_type: "application/esl"`):
```esl
namespace demo = "urn:eigenius:demo";
resource demo:rex : demo:Dog { demo:name = "Rex"; }
```

EigenQL:
```
USING "urn:eigenius:demo:Dog"
MATCH "urn:eigenius:demo:Dog"(?d) { "urn:eigenius:demo:name": ?name }
RETURN [] { name: ?name }
```

## Workflow recipes

**Load and query:**
1. `eigenius_load` with the JSON-stringified Eigon-JSON array (must include `is_a`).
2. `eigenius_query` with EigenQL referencing the loaded class.
3. Decode rows from the returned JSON — keys are synthesized IRIs, see pitfalls.

**Run a program:**
1. `eigenius_validate_program` first (cheap; surfaces type errors without committing).
2. `eigenius_run_program` (or `_by_iri` if both program and input are already in the chain).
3. Inspect provenance via `eigenius_inspect` on the returned `trace_iri`.

**Discover and use an institution:**
1. `eigenius_list_institutions` — surfaces each institution's QueryClasses + comorphisms.
2. Pick an OnDemand QueryClass; use its `query_class` (input class) and `result_class`.
3. In EigenQL: `FIBER <institution_iri>(<input>) AS ?out INTO "<output_iri>"`.

**Generate structured LLM output:**
1. `eigenius_get_schema` for the target class.
2. Pass the JSON Schema to your LLM's JSON-mode (or use the platform's `CompleteJson` component if running a program).

## Pitfalls

- **`is_a` is mandatory.** The validator rejects resources without a class. The current parser does accept empty arrays as a value shape but the structural validator still requires `is_a` to be non-empty and to resolve.
- **Query result row keys are synthesized IRIs**, not the short names declared in `RETURN`. Expect keys like `urn:eigenius:query:gen:<hex>:row:<name>`. Match by position or fetch the row class via `eigenius_inspect` to recover short names.
- **`eigenius_load` takes a JSON *string*** — the proto field is bytes. If constructing programmatically, `JSON.stringify` first.
- **Branch / tag / task / GC operations need a persistent backend.** In-memory kernels (no `--db`) reject them with `failed_precondition`. Use the docker compose stack or `eigenius serve --db <path>`.
- **`eigenius_run_program` requires program and input to share content type.** Use `eigenius_run_program_by_iri` when both are already loaded — sidesteps the issue.
- **D41 commit pipeline returns up to three layers per load.** The top-level `layer_id` always points at the *user* layer, but `committedLayers` may also include an audit-provenance sibling and an institution-classes child. Match on `role`, not array position.
- **Cascade tombstones (D41).** With `policy: "cascadeTombstone"`, lower-layer IRIs invalidated by the new commit get tombstoned iteratively. Surface this to the user — silent cascades can be surprising.

## Prerequisites

The MCP server `eigenius` must be configured (`claude mcp list` should show it as connected) and the orchestrator (`http://localhost:8080/mcp`) must be reachable. The orchestrator in turn needs the kernel running on `localhost:50051`. The simplest setup:

```bash
docker compose up -d   # kernel + orchestrator + MCP route
```

For real LLM responses, set `ANTHROPIC_API_KEY` instead of the mock flag.

## Reasoning & provenance surface (the epistemic stack)

The chain constructs the **`reasoning`** skill composes. Read the linked spec /
ontology when authoring witnessed propositions — don't memorize the shapes.

- **Epistemic status + provenance** — the `reflection` ontology
  (`ontologies/reflection/reflection-ontology.json`): `ObservedResource` /
  `DeclaredResource` / `DerivedResource`; `DeclarationTrace` / `ProgramTrace`; and
  `reflection:canonical_proposition` (the proposition a resource carries). Trace
  schema: [D6b](https://github.com/eigenius/eigenius/blob/main/docs/design/d6b-reasoning-trace-schema.md).
- **Verified reasoning** — the `reasoning` ontology (`ontologies/reasoning/`) + the
  justification-logic institution
  ([D39](https://github.com/eigenius/eigenius/blob/main/docs/design/d39-justification-logic.md),
  guide [reasoning-institution/](https://github.com/eigenius/eigenius/tree/main/docs/guides/platform/reasoning-institution)):
  a `ReasoningSentence` carries `justification` + `certificate`; the certificate
  type-checks against `JustifiedBy(justification, proposition)` via
  `DerivedEvidence`/`DeclaredEvidence`/`VerifiedEvidence`/`App` and the
  `derived()/declared()/verified()/app()` certificate constructors.
- **Witness index** — [D49](https://github.com/eigenius/eigenius/blob/main/docs/design/d49-chainwitness-machinery.md):
  how `IsObservedAs`/`IsDeclaredAs`/`IsDerivedAs`/`IsVerifiedAs` witnesses are
  admitted per layer and consumed by certificates.
- **Lemma citation** — [D54](https://github.com/eigenius/eigenius/blob/main/docs/design/d54-reasoning-lemma-citation.md):
  a `Holds` `ReasoningSentence` is citable as a lemma (`verified(<iri>, P)`) →
  layered proofs.
- **The commit gate (fail-closed).** AutoOnLoad **rejects** a layer that adds a
  `Fails` `ReasoningSentence`, so a later lemma citation of it can't be unsound —
  this is what makes "you can't record an unwitnessed conclusion" structural.
- **Anchors (third-party knowledge)** — the `reference` ontology
  (`ontologies/reference/reference.esl`): `reference:Reference` (a bibliographic
  work) + CiTO-typed `reference:Citation` carrying an imported claim. Use real,
  resolvable DOIs/PMIDs — never fabricated.
- **Recompute / external evidence** — the statistics institution
  ([D52](https://github.com/eigenius/eigenius/blob/main/docs/design/d52-measurement-statistics-institution.md),
  guide [statistics-institution/](https://github.com/eigenius/eigenius/tree/main/docs/guides/platform/statistics-institution)):
  `SampleSet` + `StatisticalAnalysisPlan` → an `IsDerivedAs` result; large external
  files + the `ingest` ontology ([D53](https://github.com/eigenius/eigenius/blob/main/docs/design/d53-large-data-tracking.md));
  wrapped external tools via the R runtime
  ([D55](https://github.com/eigenius/eigenius/blob/main/docs/design/d55-r-language-runtime.md) /
  [D56](https://github.com/eigenius/eigenius/blob/main/docs/design/d56-component-execution-and-derivation-materialization.md),
  the `RunRuntimeScript` component).

## Going deeper

**Guides** (worked examples, chapter-by-chapter):
- ESL — [https://eigenius.github.io/eigenius/esl/](https://eigenius.github.io/eigenius/esl/)
- EigenQL — [https://eigenius.github.io/eigenius/eigenql/](https://eigenius.github.io/eigenius/eigenql/)
- Formula language — [https://eigenius.github.io/eigenius/formula/](https://eigenius.github.io/eigenius/formula/)
- Platform (notebook, CLI, deployment, runtime substrate, Julia / Lean walkthroughs, SDK) — [https://eigenius.github.io/eigenius/platform/](https://eigenius.github.io/eigenius/platform/)
- References / bibliography — [https://eigenius.github.io/eigenius/references/](https://eigenius.github.io/eigenius/references/)

**Design documents** (specs; in-repo only):
- Index — [docs/design/](https://github.com/eigenius/eigenius/tree/main/docs/design)
- [D1](https://github.com/eigenius/eigenius/blob/main/docs/design/d1-eigon-serialization-format.md) Eigon serialization · [D2](https://github.com/eigenius/eigenius/blob/main/docs/design/d2-eigenql-specification.md) EigenQL · [D3](https://github.com/eigenius/eigenius/blob/main/docs/design/d3-program-model.md) Program model · [D7](https://github.com/eigenius/eigenius/blob/main/docs/design/d7-esl-surface-syntax.md) ESL syntax
- [D14](https://github.com/eigenius/eigenius/blob/main/docs/design/d14-institution-realisation.md) Institution realisation · [D22](https://github.com/eigenius/eigenius/blob/main/docs/design/d22-notebook-and-typescript-sdk.md) Notebook + SDK · [D41](https://github.com/eigenius/eigenius/blob/main/docs/design/d41-commit-pipeline.md) Commit pipeline
- [D26](https://github.com/eigenius/eigenius/blob/main/docs/design/d26-runtime-substrate.md) Runtime substrate · [D27](https://github.com/eigenius/eigenius/blob/main/docs/design/d27-julia-institutions.md) Julia institutions · [D28](https://github.com/eigenius/eigenius/blob/main/docs/design/d28-lean-4-as-institution.md) Lean verification

**Source of truth for the bootstrap ontologies** (always read these rather than memorize):
- Core platform: `ontologies/core/` `ontologies/program/` `ontologies/reflection/` `ontologies/institution/` `ontologies/formulas/` `ontologies/runtime/`
- Reasoning stack (the `reasoning` skill): `ontologies/reasoning/` `ontologies/reference/` `ontologies/statistics/` `ontologies/ingest/`
