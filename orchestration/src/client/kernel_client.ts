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
   * Execute an EigenQL query. Collects all streamed results.
   */
  async query(eigenql: string): Promise<Uint8Array[]> {
    const results: Uint8Array[] = [];
    for await (
      const result of this.client.query(
        create(QueryRequestSchema, { eigenql }),
      )
    ) {
      results.push(result.resource);
    }
    return results;
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
