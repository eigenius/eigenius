/**
 * Connect RPC client for the Eigenius kernel service.
 *
 * Uses @connectrpc/connect with buf-generated types from proto/eigenius.proto.
 * Provides typed methods for all kernel operations.
 *
 * See design doc D5 for the API spec.
 */

import { createClient } from "@connectrpc/connect";
import { createGrpcTransport } from "@connectrpc/connect-node";
import { create } from "@bufbuild/protobuf";
import {
  EigeniusKernel,
  HealthRequestSchema,
  type HealthResponse,
  InspectRequestSchema,
  type InspectResponse,
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

export type {
  HealthResponse,
  InspectResponse,
  LoadResponse,
  ReflectResponse,
  RunProgramResponse,
  ValidateProgramResponse,
  ValidationError,
};

const TEXT_ENCODER = new TextEncoder();

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
    const transport = createGrpcTransport({
      baseUrl: endpoint,
    });
    this.client = createClient(EigeniusKernel, transport);
  }

  /** Get the configured endpoint. */
  getEndpoint(): string {
    return this.endpoint;
  }

  /**
   * Load resources into the kernel's working layer.
   */
  async load(
    resourcesJson: string,
    autoCommit = true,
  ): Promise<LoadResponse> {
    return await this.client.load(
      create(LoadRequestSchema, {
        resources: TEXT_ENCODER.encode(resourcesJson),
        contentType: "application/eigon+json",
        autoCommit,
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
   * downstream consumers (notably the WASM `query-access.query` host
   * import, which contracts for `list<list<u8>>`) don't have to walk
   * the document themselves.
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
        program: TEXT_ENCODER.encode(programJson),
        contentType: "application/eigon+json",
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
        program: TEXT_ENCODER.encode(programJson),
        input: TEXT_ENCODER.encode(inputJson),
        contentType: "application/eigon+json",
      }),
    );
  }

  /**
   * Record a reasoning trace.
   */
  async reflect(traceJson: string): Promise<ReflectResponse> {
    return await this.client.reflect(
      create(ReflectRequestSchema, {
        trace: TEXT_ENCODER.encode(traceJson),
        contentType: "application/eigon+json",
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
