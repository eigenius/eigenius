// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/**
 * MCP Server — LLM tool-use surface for the Eigenius kernel.
 *
 * Exposes a curated subset of the kernel's gRPC RPCs as MCP tools so that
 * LLM agents (Claude Desktop, IDE-integrated agents, etc.) can drive the
 * platform — query the knowledge graph, run programs, inspect provenance,
 * and observe chain state.
 *
 * Scope:
 *   - Explore  — query, inspect, list_branches / tags / institutions,
 *                get_schema, layer_topology
 *   - Mutate   — load, validate_program, run_program, run_program_by_iri
 *   - Observe  — health, list_tasks, get_task_status
 *
 * Out of scope (deliberately): branch / tag mutation, merge submission,
 * GC, consolidation, task cancellation. Those are stateful multi-turn
 * flows or destructive ops that belong to the notebook UI or an operator,
 * not a single-turn agent invocation.
 *
 * Two transports are wired:
 *  - HTTP — mounted at `/mcp` on the orchestrator's HTTP port via
 *    `createMcpHttpHandler` ([`./http.ts`]). The right path for the
 *    docker compose stack; clients connect at `http://localhost:8080/mcp`.
 *  - stdio — `startStdioServer` (below); entry point at
 *    [`../mcp_main.ts`], runnable as `deno task mcp`. The right path
 *    for kernel-on-host development without bringing up the orchestrator
 *    container.
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { create, toJson } from "@bufbuild/protobuf";
import { decode as cborDecode } from "cbor-x";
import type { KernelClient, LoadPolicy } from "../client/kernel_client.ts";
import {
  GetSchemaRequestSchema,
  GetSchemaResponseSchema,
  GetTaskStatusRequestSchema,
  GetTaskStatusResponseSchema,
  HealthRequestSchema,
  HealthResponseSchema,
  InspectRequestSchema,
  LayerTopologyRequestSchema,
  LayerTopologyResponseSchema,
  ListBranchesRequestSchema,
  ListBranchesResponseSchema,
  ListInstitutionsRequestSchema,
  ListInstitutionsResponseSchema,
  ListTagsRequestSchema,
  ListTagsResponseSchema,
  ListTasksRequestSchema,
  ListTasksResponseSchema,
  LoadResponseSchema,
  QueryRequestSchema,
  RunProgramByIriRequestSchema,
  RunProgramResponseSchema,
  ValidateProgramResponseSchema,
} from "../gen/eigenius_pb.ts";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";

const SERVER_NAME = "eigenius";
const SERVER_VERSION = "0.2.0";

/**
 * Create and configure the Eigenius MCP server.
 *
 * Each tool wraps one (or, in a few cases, one-and-a-half) kernel RPCs.
 * Responses are returned as a single text content block carrying the
 * JSON-serialised proto response (enums rendered as their string names
 * via `toJson`), with `isError: true` set on RPC-level failures.
 */
export function createMcpServer(client: KernelClient): McpServer {
  const server = new McpServer({
    name: SERVER_NAME,
    version: SERVER_VERSION,
  });

  // ===========================================================================
  // Explore — read-only navigation of the chain.
  // ===========================================================================

  server.tool(
    "eigenius_query",
    "Execute an EigenQL query against the Eigenius knowledge graph. " +
      "Returns the rows as decoded JS objects (Eigon-CBOR → JSON), plus " +
      "any FIBER ... INTO output IRIs and the branch-CAS outcome. Optionally " +
      "pin reads to a specific branch or layer for time-travel queries.",
    {
      eigenql: z.string().describe(
        "EigenQL program string. See docs/guides/eigenql/ for the language reference.",
      ),
      branch: z.string().optional().describe(
        "Branch to pin reads to (defaults to 'main').",
      ),
      atLayer: z.string().optional().describe(
        "Hex-encoded LayerId to pin reads to. Mutually exclusive with `branch`.",
      ),
    },
    async (args: { eigenql: string; branch?: string; atLayer?: string }) => {
      const resp = await client.raw.query(create(QueryRequestSchema, {
        eigenql: args.eigenql,
        branch: args.branch ?? "",
        atLayer: args.atLayer ?? "",
      }));
      if (!resp.success) {
        return errorResult(`Query failed: ${resp.error}`);
      }
      const rows = resp.document.length > 0
        ? decodeQueryRows(resp.document)
        : [];
      return jsonResult({
        rows,
        rowCount: rows.length,
        outputResourceIris: resp.outputResourceIris,
        branchAdvanced: resp.branchAdvanced,
        merge: resp.merge ? toJsonSafe("MergeInfo", resp.merge) : null,
      });
    },
  );

  server.tool(
    "eigenius_inspect",
    "Resolve a single resource by its IRI. Returns the resource decoded " +
      "from Eigon-CBOR to a JSON object (property keys are full IRIs).",
    {
      iri: z.string().describe("IRI of the resource to inspect."),
      branch: z.string().optional().describe(
        "Branch to read from (defaults to 'main').",
      ),
      atLayer: z.string().optional().describe(
        "Hex-encoded LayerId to pin the read to. Mutually exclusive with `branch`.",
      ),
    },
    async (args: { iri: string; branch?: string; atLayer?: string }) => {
      const resp = await client.raw.inspect(create(InspectRequestSchema, {
        iri: args.iri,
        branch: args.branch ?? "",
        atLayer: args.atLayer ?? "",
      }));
      if (!resp.found) {
        return jsonResult({ found: false, iri: args.iri });
      }
      const decoded = cborDecode(resp.resource);
      return jsonResult({ found: true, iri: args.iri, resource: decoded });
    },
  );

  server.tool(
    "eigenius_list_branches",
    "List every branch ref with its head LayerId and head commit time.",
    {},
    async () => {
      const resp = await client.raw.listBranches(
        create(ListBranchesRequestSchema, {}),
      );
      return jsonResult(
        toJson(ListBranchesResponseSchema, resp),
      );
    },
  );

  server.tool(
    "eigenius_list_tags",
    "List every immutable tag ref with its target LayerId.",
    {},
    async () => {
      const resp = await client.raw.listTags(create(ListTagsRequestSchema, {}));
      return jsonResult(toJson(ListTagsResponseSchema, resp));
    },
  );

  server.tool(
    "eigenius_list_institutions",
    "List indexed D14 institutions with their QueryClasses, comorphisms, " +
      "runtime kind, and required RuntimeEnvironment (if external). Use " +
      "this to discover what FIBER queries and AutoOnLoad gates are " +
      "available on the chain.",
    {
      atLayer: z.string().optional().describe(
        "Hex-encoded LayerId to read against (defaults to the active top).",
      ),
    },
    async (args: { atLayer?: string }) => {
      const resp = await client.raw.listInstitutions(
        create(ListInstitutionsRequestSchema, { atLayer: args.atLayer ?? "" }),
      );
      return jsonResult(toJson(ListInstitutionsResponseSchema, resp));
    },
  );

  server.tool(
    "eigenius_get_schema",
    "Generate a JSON Schema for an ontology class, derived from the class's " +
      "required / recommended properties walked through the layer chain. " +
      "Useful for grounding LLM JSON generation in the typed shape.",
    {
      classIri: z.string().describe(
        "IRI of the class to generate a schema for.",
      ),
      atLayer: z.string().optional().describe(
        "Hex-encoded LayerId to read against (defaults to the active top).",
      ),
    },
    async (args: { classIri: string; atLayer?: string }) => {
      const resp = await client.raw.getSchema(
        create(GetSchemaRequestSchema, {
          classIri: args.classIri,
          atLayer: args.atLayer ?? "",
        }),
      );
      if (!resp.success) {
        return errorResult(`GetSchema failed: ${resp.error}`);
      }
      // jsonSchema is itself a JSON string — embed it as parsed JSON so the
      // agent sees structure, not an escaped string.
      const schema = JSON.parse(resp.jsonSchema);
      const base = toJson(GetSchemaResponseSchema, resp) as Record<
        string,
        unknown
      >;
      return jsonResult({ ...base, jsonSchema: schema });
    },
  );

  server.tool(
    "eigenius_layer_topology",
    "Walk the layer chain and return a graph (nodes + edges) summarising " +
      "what each layer contributed. Use this to orient before deeper " +
      "queries — see which classes / properties / institutions live where.",
    {
      rootLayer: z.string().optional().describe(
        "Hex-encoded LayerId to root the walk at (defaults to the active top).",
      ),
      maxDepth: z.number().int().nonnegative().optional().describe(
        "Maximum parent-pointer hops from `rootLayer`. 0 = unlimited.",
      ),
      includeResources: z.boolean().optional().describe(
        "When true, emit a node per resource. When false (default), only " +
          "Class / Property / Institution become nodes.",
      ),
    },
    async (
      args: {
        rootLayer?: string;
        maxDepth?: number;
        includeResources?: boolean;
      },
    ) => {
      const resp = await client.raw.layerTopology(
        create(LayerTopologyRequestSchema, {
          rootLayer: args.rootLayer ?? "",
          maxDepth: args.maxDepth ?? 0,
          includeResources: args.includeResources ?? false,
        }),
      );
      return jsonResult(toJson(LayerTopologyResponseSchema, resp));
    },
  );

  // ===========================================================================
  // Mutate — commit-shaped operations.
  // ===========================================================================

  server.tool(
    "eigenius_load",
    "Load Eigon-JSON resources into the kernel as a new chain layer. " +
      "The D41 commit pipeline runs structural validation, then either " +
      "rejects on retroactive violations (`policy: 'reject'`, default) or " +
      "iteratively tombstones violating lower-layer IRIs (`policy: " +
      "'cascadeTombstone'`). The response surfaces per-layer outcomes — " +
      "the user layer, optional audit-provenance sibling, and optional " +
      "institution-classes child — each with its own branch-CAS outcome.",
    {
      json: z.string().describe(
        "Eigon-JSON: a single resource (object) or a document (array of resources).",
      ),
      autoCommit: z.boolean().optional().describe(
        "Commit after successful validation (default: true).",
      ),
      branch: z.string().optional().describe(
        "Branch to commit into (defaults to 'main'). Must already exist.",
      ),
      policy: z.enum(["reject", "cascadeTombstone"]).optional().describe(
        "Commit policy for retroactive validation (D41 §3.3). " +
          "'reject' (default) fails the commit on any violation; " +
          "'cascadeTombstone' iteratively tombstones violating lower-layer IRIs.",
      ),
      maxViolations: z.number().int().nonnegative().optional().describe(
        "When policy='reject', cap on the number of ValidationError entries " +
          "returned. 0 = kernel default (currently 100). The true count is " +
          "always reported via `totalViolations`.",
      ),
      explicitTombstones: z.array(z.string()).optional().describe(
        "IRIs the caller wants tombstoned as part of this commit (D41 §10.1).",
      ),
    },
    async (
      args: {
        json: string;
        autoCommit?: boolean;
        branch?: string;
        policy?: "reject" | "cascadeTombstone";
        maxViolations?: number;
        explicitTombstones?: string[];
      },
    ) => {
      const policy: LoadPolicy | undefined = args.policy === "reject"
        ? { kind: "reject", maxViolations: args.maxViolations }
        : args.policy === "cascadeTombstone"
        ? { kind: "cascadeTombstone" }
        : undefined;

      const resp = await client.load(args.json, {
        autoCommit: args.autoCommit ?? true,
        branch: args.branch,
        policy,
        explicitTombstones: args.explicitTombstones,
      });
      return jsonResult(toJson(LoadResponseSchema, resp));
    },
  );

  server.tool(
    "eigenius_validate_program",
    "Type-check a program (Eigon-JSON) against the active layer chain " +
      "without executing it. Returns the inferred program type and any " +
      "validation errors.",
    {
      json: z.string().describe(
        "Program resource as Eigon-JSON (single object or document).",
      ),
    },
    async (args: { json: string }) => {
      const resp = await client.validateProgram(args.json);
      return jsonResult(toJson(ValidateProgramResponseSchema, resp));
    },
  );

  server.tool(
    "eigenius_run_program",
    "Execute a program with input data, both supplied inline as " +
      "Eigon-JSON. Returns the output resource (decoded from CBOR), the " +
      "trace IRI, the task ID (when a persistent backend is attached), " +
      "and any chain-resident output IRIs from FIBER reify steps.",
    {
      programJson: z.string().describe("Program resource as Eigon-JSON."),
      inputJson: z.string().describe("Input resource as Eigon-JSON."),
    },
    async (args: { programJson: string; inputJson: string }) => {
      const resp = await client.runProgram(args.programJson, args.inputJson);
      const json = toJson(RunProgramResponseSchema, resp) as Record<
        string,
        unknown
      >;
      // `output` is base64-encoded CBOR in the proto-JSON shape. Decode
      // for the agent so it sees the actual result resource, not bytes.
      if (resp.success && resp.output.length > 0) {
        try {
          json.output = cborDecode(resp.output);
        } catch (e) {
          json.outputDecodeError = (e as Error).message;
        }
      }
      return jsonResult(json);
    },
  );

  server.tool(
    "eigenius_run_program_by_iri",
    "Execute a program already loaded into the chain, identified by IRI, " +
      "against an input also in the chain. Avoids re-shipping bytes — the " +
      "natural fit when the agent has just loaded both. Both reads can be " +
      "pinned to a specific LayerId.",
    {
      programIri: z.string().describe("IRI of the program resource."),
      inputIri: z.string().describe("IRI of the input resource."),
      atLayer: z.string().optional().describe(
        "Hex-encoded LayerId to pin both reads to (defaults to active top).",
      ),
      branch: z.string().optional().describe(
        "Branch the trace layer commits into (defaults to 'main').",
      ),
    },
    async (
      args: {
        programIri: string;
        inputIri: string;
        atLayer?: string;
        branch?: string;
      },
    ) => {
      const resp = await client.raw.runProgramByIri(
        create(RunProgramByIriRequestSchema, {
          programIri: args.programIri,
          inputIri: args.inputIri,
          atLayer: args.atLayer ?? "",
          branch: args.branch ?? "",
        }),
      );
      const json = toJson(RunProgramResponseSchema, resp) as Record<
        string,
        unknown
      >;
      if (resp.success && resp.output.length > 0) {
        try {
          json.output = cborDecode(resp.output);
        } catch (e) {
          json.outputDecodeError = (e as Error).message;
        }
      }
      return jsonResult(json);
    },
  );

  // ===========================================================================
  // Observe — orientation and task state.
  // ===========================================================================

  server.tool(
    "eigenius_health",
    "Health check the kernel. Returns version, layer / resource counts, and " +
      "the D21 resume-sweep state.",
    {},
    async () => {
      const resp = await client.raw.health(create(HealthRequestSchema, {}));
      return jsonResult(toJson(HealthResponseSchema, resp));
    },
  );

  server.tool(
    "eigenius_list_tasks",
    "List every D21 task in the session — running, suspended, completed, " +
      "or failed. Empty when no persistent backend is attached.",
    {},
    async () => {
      const resp = await client.raw.listTasks(
        create(ListTasksRequestSchema, {}),
      );
      return jsonResult(toJson(ListTasksResponseSchema, resp));
    },
  );

  server.tool(
    "eigenius_get_task_status",
    "Look up a specific task's status, pinned LayerId, and result-layer " +
      "head (when completed).",
    {
      taskId: z.string().describe("Task UUID."),
    },
    async (args: { taskId: string }) => {
      const resp = await client.raw.getTaskStatus(
        create(GetTaskStatusRequestSchema, { taskId: args.taskId }),
      );
      return jsonResult(toJson(GetTaskStatusResponseSchema, resp));
    },
  );

  return server;
}

/**
 * Start the MCP server with stdio transport. The standard wiring for
 * Claude Desktop / similar local agent hosts: the host launches the
 * orchestrator process with stdin/stdout piped, and the MCP protocol
 * runs over those two FDs.
 */
export async function startStdioServer(server: McpServer): Promise<void> {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  log.info(operation.MCP_SERVER_START, "MCP server connected", {
    transport: "stdio",
    name: SERVER_NAME,
    version: SERVER_VERSION,
  });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const IS_A = "urn:eigenius:core:is_a";
const RESULT_SET_CLASS = "urn:eigenius:query:ResultSet";
const ROWS_PROP = "urn:eigenius:query:rows";

/**
 * Walk an Eigon-CBOR query document and return each row as a decoded JS
 * object. Mirrors `extractRowBytes` in `client/kernel_client.ts` but
 * returns decoded values rather than re-encoded bytes — MCP consumers
 * want JSON, not CBOR.
 */
function decodeQueryRows(documentBytes: Uint8Array): unknown[] {
  // deno-lint-ignore no-explicit-any
  const decoded: any = cborDecode(documentBytes);
  const resources = Array.isArray(decoded) ? decoded : [decoded];
  const resultSet = resources.find((r) =>
    r && Array.isArray(r[IS_A]) && r[IS_A].includes(RESULT_SET_CLASS)
  );
  if (!resultSet) return [];
  const rowsField = resultSet[ROWS_PROP];
  if (!Array.isArray(rowsField)) return [];
  return rowsField.map((entry) => {
    // Rows are either embedded resource objects or IRI references; in the
    // EigenQL result document they're always embedded.
    if (typeof entry === "object" && entry !== null) return entry;
    return { ref: entry };
  });
}

/** Wrap a value as a single text-content JSON response. */
function jsonResult(value: unknown) {
  return {
    content: [{
      type: "text" as const,
      text: JSON.stringify(value, replaceBigint, 2),
    }],
  };
}

/** Wrap an error message as an MCP `isError: true` response. */
function errorResult(message: string) {
  return {
    content: [{
      type: "text" as const,
      text: message,
    }],
    isError: true,
  };
}

/** JSON.stringify replacer that handles bigint (proto uint64 / int64). */
function replaceBigint(_key: string, value: unknown): unknown {
  return typeof value === "bigint" ? value.toString() : value;
}

/**
 * Best-effort `toJson` wrapper for embedded sub-messages we don't have
 * a schema import handy for (e.g. `MergeInfo` nested inside a
 * QueryResponse — toJson at the parent level handles it, but we want
 * to project it standalone for the Query tool). Falls back to a manual
 * object snapshot.
 */
// deno-lint-ignore no-explicit-any
function toJsonSafe(_label: string, message: any): unknown {
  if (!message) return null;
  // The buf-generated message has typeName-keyed serialiser metadata
  // we'd need to look up. Cheaper to do a plain spread — the enum
  // numeric values won't be human-readable, but Outcome semantics are
  // documented in proto comments and the agent can match by number.
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(message)) {
    if (typeof v === "bigint") out[k] = v.toString();
    else out[k] = v;
  }
  return out;
}
