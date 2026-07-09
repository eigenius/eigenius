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
 * Connect RPC client for the Eigenius kernel service.
 *
 * Uses @connectrpc/connect with buf-generated types from proto/eigenius.proto.
 * Provides typed methods for all kernel operations.
 *
 * See design doc D5 for the API spec.
 */

import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-node";
import { create } from "@bufbuild/protobuf";
import {
  type CommitPolicy,
  CommitPolicy_CascadeTombstoneSchema,
  CommitPolicy_RejectSchema,
  CommitPolicySchema,
  type CommittedLayer,
  EigeniusKernel,
  HealthRequestSchema,
  type HealthResponse,
  InspectRequestSchema,
  type InspectResponse,
  LayerRole,
  LayerTopologyRequestSchema,
  type LayerTopologyResponse,
  LoadRequestSchema,
  type LoadResponse,
  QueryRequestSchema,
  ReflectRequestSchema,
  type ReflectResponse,
  RunProgramRequestSchema,
  type RunProgramResponse,
  ValidateProgramRequestSchema,
  type ValidateProgramResponse,
  type ValidationError,
} from "../gen/eigenius_pb.ts";
import { encodeResource } from "../codec/cbor.ts";

export type {
  CommitPolicy,
  CommittedLayer,
  HealthResponse,
  InspectResponse,
  LayerTopologyResponse,
  LoadResponse,
  ReflectResponse,
  RunProgramResponse,
  ValidateProgramResponse,
  ValidationError,
};

export { LayerRole };

/**
 * Commit-policy shapes accepted by `KernelClient.load`. Maps to the
 * proto `CommitPolicy` oneof (D41 §3.3, §8). Absent or `{ kind: "reject" }`
 * with no `maxViolations` defers to the server-side default
 * (`Reject{ max_violations: 100 }`).
 */
export type LoadPolicy =
  | { kind: "reject"; maxViolations?: number }
  | { kind: "cascadeTombstone" };

function policyToProto(
  policy: LoadPolicy | undefined,
): CommitPolicy | undefined {
  if (!policy) return undefined;
  if (policy.kind === "reject") {
    return create(CommitPolicySchema, {
      variant: {
        case: "reject",
        value: create(CommitPolicy_RejectSchema, {
          maxViolations: policy.maxViolations ?? 0,
        }),
      },
    });
  }
  return create(CommitPolicySchema, {
    variant: {
      case: "cascadeTombstone",
      value: create(CommitPolicy_CascadeTombstoneSchema, {}),
    },
  });
}

const CONTENT_TYPE_CBOR = "application/eigon+cbor";

/**
 * Phase 18e: re-encode a JSON-string-shaped Eigon payload as Eigon-CBOR
 * for transport to the kernel. The kernel server's `parse_resources`
 * branches on `content_type` and accepts both; this client always sends
 * CBOR. Parser handles both single-resource (object) and document
 * (array) shapes — kernel-side `parse_document` accepts either.
 *
 * Caveat: this conversion uses cbor-x's default encoding, which does
 * NOT wrap `Value::Json` payloads with `EIGENIUS_JSON_TAG`. For the
 * RPCs flipped here (Load / ValidateProgram / RunProgram / Reflect),
 * resources have not historically used `data_type: json` properties so
 * the limitation is invisible. If a future caller needs to round-trip
 * a JSON-typed property through these RPCs, extend this helper to use
 * an Eigon-aware encoder that mirrors the kernel's
 * `eigon_cbor::value_to_cbor` Json branch.
 */
function jsonStringToEigonCbor(jsonString: string): Uint8Array {
  return encodeResource(JSON.parse(jsonString));
}

/**
 * Client for the Eigenius kernel gRPC service.
 *
 * Uses Connect RPC transport to communicate with the kernel's tonic
 * gRPC server over HTTP/2.
 */
export class KernelClient {
  private client: ReturnType<typeof createClient<typeof EigeniusKernel>>;
  private endpoint: string;

  constructor(endpoint: string) {
    this.endpoint = endpoint;
    // Use gRPC-Web over HTTP/1.1 instead of native gRPC over HTTP/2.
    // The kernel speaks both — it's wrapped in `tonic_web::enable(...)`
    // at the serve entry point, with `accept_http1(true)` on the
    // tonic builder. The reason we don't use `createGrpcTransport`
    // (HTTP/2 native): Deno's `node:http2` polyfill that connect-node
    // depends on has multi-second per-request latency and session-
    // reuse hangs, making notebook-driven queries unusable. gRPC-Web
    // routes through Deno's native `fetch()` (HTTP/1.1) which works
    // correctly. Same RPC surface, same wire-level semantics; the
    // only difference is the transport encoding (gRPC-Web's
    // length-prefixed frames over HTTP/1.1 chunked vs gRPC's HTTP/2
    // streams).
    const transport = createGrpcWebTransport({
      baseUrl: endpoint,
      // Force HTTP/1.1 — connect-node v2 defaults to HTTP/2 for
      // gRPC-Web too (via the same `node:http2` polyfill), which
      // defeats the purpose of switching. With `"1.1"`, the transport
      // uses `node:http`'s `request()` which Deno polyfills cleanly.
      httpVersion: "1.1",
    });
    this.client = createClient(EigeniusKernel, transport);
  }

  /** Get the configured endpoint. */
  getEndpoint(): string {
    return this.endpoint;
  }

  /**
   * Raw Connect client for direct request/response passthrough. The
   * higher-level methods on this class adapt the kernel's wire types
   * into JS-friendly shapes (e.g. `query()` returns `Uint8Array[]`
   * extracted from the result document); the orchestrator's notebook
   * passthrough wants the unmodified request/response, so it uses
   * this accessor instead.
   */
  get raw(): ReturnType<typeof createClient<typeof EigeniusKernel>> {
    return this.client;
  }

  /**
   * Load resources into the kernel's working layer.
   *
   * `policy` controls retroactive-validation behaviour (D41 §3.3, §8).
   * Omit for the server-side default (`Reject{ max_violations: 100 }`).
   * `explicitTombstones` tombstones the listed IRIs as part of the same
   * commit, applied to the initial user-layer builder before retroactive
   * validation runs (D41 §10.1).
   *
   * The returned `LoadResponse` carries `totalViolations` (the true
   * violation count, possibly larger than `errors.length` when the
   * policy capped it), `committedLayers` (per-layer outcomes — match
   * on `role` to find the user / audit / institution-classes layer),
   * and the existing top-level `layerId` / `branchAdvanced` / `merge`
   * fields which continue to point at the user layer.
   */
  async load(
    resourcesJson: string,
    options: {
      autoCommit?: boolean;
      branch?: string;
      policy?: LoadPolicy;
      explicitTombstones?: string[];
    } = {},
  ): Promise<LoadResponse> {
    return await this.client.load(
      create(LoadRequestSchema, {
        resources: jsonStringToEigonCbor(resourcesJson),
        contentType: CONTENT_TYPE_CBOR,
        autoCommit: options.autoCommit ?? true,
        branch: options.branch ?? "",
        policy: policyToProto(options.policy),
        explicitTombstones: options.explicitTombstones ?? [],
      }),
    );
  }

  /**
   * Resolve a resource by IRI.
   */
  async inspect(iri: string): Promise<InspectResponse> {
    return await this.client.inspect(
      create(InspectRequestSchema, { iri }),
    );
  }

  /**
   * Execute an EigenQL query. The kernel returns an Eigon document
   * (see D2 Appendix A) — we extract the embedded row resources from
   * the ResultSet and return them individually as CBOR byte arrays so
   * downstream consumers that contract for `list<list<u8>>` don't have
   * to walk the document themselves.
   *
   * Rows keep their synthesized Property IRI keys; callers that want
   * the short-name view should consult the ResultSet's row class (see
   * the full document via gRPC if needed) or use this method's result
   * in combination with the property list.
   */
  async query(eigenql: string): Promise<Uint8Array[]> {
    const resp = await this.client.query(
      create(QueryRequestSchema, { eigenql }),
    );
    if (!resp.success) {
      throw new Error(`Query failed: ${resp.error}`);
    }
    if (resp.document.length === 0) {
      return [];
    }
    return extractRowBytes(resp.document);
  }

  /**
   * Type-check a program against the kernel's layer chain.
   */
  async validateProgram(
    programJson: string,
  ): Promise<ValidateProgramResponse> {
    return await this.client.validateProgram(
      create(ValidateProgramRequestSchema, {
        program: jsonStringToEigonCbor(programJson),
        contentType: CONTENT_TYPE_CBOR,
      }),
    );
  }

  /**
   * Execute a program with input data.
   */
  async runProgram(
    programJson: string,
    inputJson: string,
  ): Promise<RunProgramResponse> {
    return await this.client.runProgram(
      create(RunProgramRequestSchema, {
        program: jsonStringToEigonCbor(programJson),
        input: jsonStringToEigonCbor(inputJson),
        contentType: CONTENT_TYPE_CBOR,
      }),
    );
  }

  /**
   * Record a reasoning trace.
   */
  async reflect(traceJson: string): Promise<ReflectResponse> {
    return await this.client.reflect(
      create(ReflectRequestSchema, {
        trace: jsonStringToEigonCbor(traceJson),
        contentType: CONTENT_TYPE_CBOR,
      }),
    );
  }

  /**
   * Check kernel health.
   */
  async health(): Promise<HealthResponse> {
    return await this.client.health(
      create(HealthRequestSchema, {}),
    );
  }

  /**
   * Walk the layer chain and return a topology graph (D22 §4.2). The
   * orchestrator's NotebookService.LayerTopology proxies to this.
   *
   * @param rootLayer  Optional hex LayerId; empty = active top.
   * @param maxDepth   0 = unlimited (default).
   * @param includeResources When true, emits a node per Resource (any class).
   *   When false (default), only Class / Property / Institution become nodes;
   *   ordinary instances are aggregated into per-layer counts.
   */
  async layerTopology(
    rootLayer = "",
    maxDepth = 0,
    includeResources = false,
  ): Promise<LayerTopologyResponse> {
    return await this.client.layerTopology(
      create(LayerTopologyRequestSchema, {
        rootLayer,
        maxDepth,
        includeResources,
      }),
    );
  }
}

// ---------------------------------------------------------------------------
// Result-document row extraction
// ---------------------------------------------------------------------------

// Avoid importing the full cbor module at the top so this file stays
// usable from tests that mock the transport — cbor-x pulls in native
// code behind the scenes.
import { decode as cborDecode, encode as cborEncode } from "cbor-x";

const IS_A = "urn:eigenius:core:is_a";
const RESULT_SET_CLASS = "urn:eigenius:query:ResultSet";
const ROWS_PROP = "urn:eigenius:query:rows";

/**
 * Walk an Eigon-CBOR document (D2 Appendix A) and return each embedded
 * row as its own CBOR byte array. Returns `[]` for match-only queries.
 */
function extractRowBytes(documentBytes: Uint8Array): Uint8Array[] {
  // deno-lint-ignore no-explicit-any
  const decoded: any = cborDecode(documentBytes);
  const resources = Array.isArray(decoded) ? decoded : [decoded];

  const resultSet = resources.find((r) =>
    r && Array.isArray(r[IS_A]) && r[IS_A].includes(RESULT_SET_CLASS)
  );
  if (!resultSet) return [];

  const rows = resultSet[ROWS_PROP];
  if (!Array.isArray(rows)) return [];

  return rows.map((row) => cborEncode(row));
}
