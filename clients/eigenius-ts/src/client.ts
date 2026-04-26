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
 * `Eigen` — the TypeScript SDK's main entry point. Wraps the
 * orchestrator's `EigeniusKernel` and `NotebookService` Connect surfaces
 * in a single typed class. See D22 §5.
 *
 * Phase 1 scope: `layerTopology` only. Phase 2+ adds `inspect`, `query`,
 * `load`, `compile`, `run`, `listInstitutions` (existing kernel RPCs)
 * and any new browser-specific methods that join `NotebookService`.
 */

import {
  type Client,
  createClient,
  type Transport,
} from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import { create } from "@bufbuild/protobuf";
import {
  EigeniusKernel,
  type HealthResponse,
  HealthRequestSchema,
  type InspectResponse,
  InspectRequestSchema,
  type InstitutionInfo,
  type LayerTopologyResponse,
  LayerTopologyRequestSchema,
  ListInstitutionsRequestSchema,
  type LoadResponse,
  LoadRequestSchema,
  NotebookService,
  type QueryResponse,
  QueryRequestSchema,
  type RunProgramResponse,
  RunProgramRequestSchema,
  RunProgramByIriRequestSchema,
  type ValidateProgramResponse,
  ValidateProgramRequestSchema,
  type ValidationError,
} from "../generated/eigenius_pb.ts";

// Re-export wire types so consumers don't have to reach into generated/.
export type {
  HealthResponse,
  InspectResponse,
  InstitutionInfo,
  LayerTopologyResponse,
  LoadResponse,
  QueryResponse,
  RunProgramResponse,
  ValidateProgramResponse,
  ValidationError,
};

const TEXT_ENCODER = new TextEncoder();

/** Content type accepted by `Eigen.load` / `runProgram` / `validateProgram`. */
export type SourceContentType =
  | "application/x-esl"
  | "application/eigon+json"
  | "application/cbor";

import {
  type NotebookJson,
  notebookJsonToResources,
  type PublishOutput,
} from "./notebook.ts";

export interface EigenOptions {
  /** Orchestrator endpoint, e.g. `"http://localhost:8080"`. Required. */
  endpoint: string;

  /**
   * Optional fetch implementation override. Defaults to the global
   * `fetch`. Useful for Deno tests that want to mock the transport,
   * or for environments that need a custom interceptor.
   */
  fetch?: typeof fetch;

  /**
   * Optional bearer token. Currently unused — auth is deferred to
   * post-MVP per D22 §8.3. Kept on the type so callers can wire it
   * up now without an API change later.
   */
  bearerToken?: string;
}

export interface LayerTopologyOptions {
  /**
   * Hex-encoded `LayerId` to root the walk at. Empty / omitted = the
   * orchestrator's session active top (D21 §3.6 convention).
   */
  rootLayer?: string;

  /**
   * Maximum walk depth in parent-pointer hops. 0 (default) = unlimited.
   */
  maxDepth?: number;

  /**
   * When true, emits a node per Resource (any class). When false
   * (default), only Class / Property / Institution become nodes;
   * ordinary instances are aggregated into per-layer counts.
   */
  includeResources?: boolean;
}

export interface InspectOptions {
  /**
   * Hex-encoded `LayerId` to read against. Empty / omitted = the
   * orchestrator's session active top (D21 §3.6 convention).
   */
  atLayer?: string;
}

export interface QueryOptions {
  /**
   * Hex-encoded `LayerId` to evaluate the query against. Empty /
   * omitted = the orchestrator's session active top.
   */
  atLayer?: string;
}

export interface LoadOptions {
  /**
   * Wire format of `source`. ESL source is `application/x-esl`; an
   * Eigon-JSON document is `application/eigon+json`; CBOR is
   * `application/cbor`. The kernel compiles ESL inline when it sees
   * an esl-flavoured content type.
   */
  contentType?: SourceContentType;

  /**
   * Commit the resulting layer on success (default true). When false,
   * the kernel validates the resources against the active layer chain
   * and reports errors but does not extend the chain.
   */
  autoCommit?: boolean;
}

export interface RunProgramOptions {
  /**
   * Wire format used for *both* `program` and `input`. The current
   * `RunProgramRequest` proto carries a single content type for both
   * fields (Phase 3 limitation — see D22 §7); pass the same format for
   * both. Phase 3b adds per-field content types and an IRI-based
   * `RunProgramByIri` RPC so callers can run an already-loaded program
   * against an already-loaded input.
   */
  contentType?: SourceContentType;
}

export class Eigen {
  private readonly endpoint: string;
  private readonly transport: Transport;
  private readonly notebook: Client<typeof NotebookService>;
  private readonly kernel: Client<typeof EigeniusKernel>;

  constructor(options: EigenOptions) {
    this.endpoint = options.endpoint;
    this.transport = createConnectTransport({
      baseUrl: this.endpoint,
      fetch: options.fetch,
    });
    this.notebook = createClient(NotebookService, this.transport);
    this.kernel = createClient(EigeniusKernel, this.transport);
  }

  /** The orchestrator endpoint this client is bound to. */
  getEndpoint(): string {
    return this.endpoint;
  }

  // ------------------------------------------------------------------
  // NotebookService methods (browser-specific; only LayerTopology in MVP)
  // ------------------------------------------------------------------

  /**
   * Walk the layer chain and return a topology graph.
   *
   * The orchestrator's `NotebookService.LayerTopology` proxies to the
   * kernel's `EigeniusKernel.LayerTopology`. Returns nodes (layers and
   * Class / Property / Institution resources, plus per-resource Resource
   * nodes when `includeResources` is true) and edges (`parent_layer`,
   * `is_a`, `subclass_of`, `requires`, `recommends`, `property_ref`).
   */
  async layerTopology(
    options: LayerTopologyOptions = {},
  ): Promise<LayerTopologyResponse> {
    return await this.notebook.layerTopology(
      create(LayerTopologyRequestSchema, {
        rootLayer: options.rootLayer ?? "",
        maxDepth: options.maxDepth ?? 0,
        includeResources: options.includeResources ?? false,
      }),
    );
  }

  // ------------------------------------------------------------------
  // EigeniusKernel passthroughs (existing kernel surface; the orchestrator
  // exposes a curated subset — see eigenius_kernel_passthrough.ts).
  // ------------------------------------------------------------------

  /**
   * Resolve a resource by IRI.
   *
   * Returns the response with `found: false` if the IRI doesn't
   * resolve in the layer chain — this is not an error, just an
   * absence. The `resource` field is a CBOR-encoded Eigon resource.
   */
  async inspect(iri: string, options: InspectOptions = {}): Promise<InspectResponse> {
    return await this.kernel.inspect(
      create(InspectRequestSchema, {
        iri,
        atLayer: options.atLayer ?? "",
      }),
    );
  }

  /**
   * Execute an EigenQL query.
   *
   * Returns the kernel's response unchanged — `document` is a CBOR
   * Eigon document containing the ResultSet, its row class, and the
   * row resources (D2 Appendix A). Future SDK convenience methods
   * may decode this into typed `ResultRow` objects; for now consumers
   * decode `document` themselves with the cbor-x library or similar.
   */
  async query(eigenql: string, options: QueryOptions = {}): Promise<QueryResponse> {
    return await this.kernel.query(
      create(QueryRequestSchema, {
        eigenql,
        atLayer: options.atLayer ?? "",
      }),
    );
  }

  /**
   * List registered institutions and their declared fiber structure.
   */
  async listInstitutions(): Promise<readonly InstitutionInfo[]> {
    const response = await this.kernel.listInstitutions(
      create(ListInstitutionsRequestSchema, {}),
    );
    return response.institutions;
  }

  /**
   * Load resources into the kernel's active layer chain.
   *
   * `source` is either ESL source text (default) or an Eigon-JSON
   * document. The kernel compiles ESL inline when it sees an
   * esl-flavoured content type. On success with `autoCommit` (the
   * default), the new layer ID is returned in `LoadResponse.layerId`
   * and becomes the new session top; subsequent reads in the same
   * session see the loaded resources.
   *
   * Strings are UTF-8 encoded; pass a `Uint8Array` directly for CBOR.
   */
  async load(
    source: string | Uint8Array,
    options: LoadOptions = {},
  ): Promise<LoadResponse> {
    const contentType = options.contentType ?? "application/x-esl";
    const bytes = typeof source === "string"
      ? TEXT_ENCODER.encode(source)
      : source;
    return await this.kernel.load(
      create(LoadRequestSchema, {
        resources: bytes,
        contentType,
        autoCommit: options.autoCommit ?? true,
      }),
    );
  }

  /**
   * Type-check a program against the active layer chain.
   *
   * The program is sent inline (no IRI lookup); the kernel validates
   * stratification, totality, and component-argument shapes and
   * returns any structured ValidationErrors.
   */
  async validateProgram(
    program: string | Uint8Array,
    options: { contentType?: SourceContentType } = {},
  ): Promise<ValidateProgramResponse> {
    const contentType = options.contentType ?? "application/x-esl";
    const bytes = typeof program === "string"
      ? TEXT_ENCODER.encode(program)
      : program;
    return await this.kernel.validateProgram(
      create(ValidateProgramRequestSchema, {
        program: bytes,
        contentType,
      }),
    );
  }

  /**
   * Execute a program with input data.
   *
   * Both `program` and `input` are sent inline. Returns the program's
   * output resource as CBOR plus, when the kernel has a trace store
   * configured, the IRI of the recorded ProgramTrace. Run-time errors
   * surface as a non-zero `errors` array; the response is structured
   * (no Connect-RPC exception) so callers can render error tables.
   */
  async runProgram(
    program: string | Uint8Array,
    input: string | Uint8Array,
    options: RunProgramOptions = {},
  ): Promise<RunProgramResponse> {
    const programBytes = typeof program === "string"
      ? TEXT_ENCODER.encode(program)
      : program;
    const inputBytes = typeof input === "string"
      ? TEXT_ENCODER.encode(input)
      : input;
    // RunProgramRequest carries a single content_type covering both
    // fields. Phase 3b adds per-field content types (and an IRI-based
    // RunProgramByIri) so callers can mix ESL programs with Eigon-JSON
    // inputs naturally.
    const contentType = options.contentType ?? "application/x-esl";
    return await this.kernel.runProgram(
      create(RunProgramRequestSchema, {
        program: programBytes,
        input: inputBytes,
        contentType,
      }),
    );
  }

  /**
   * Execute a program already loaded into the active layer chain,
   * identified by IRI, against an input also identified by IRI.
   *
   * Avoids the single-content_type limitation of `runProgram` (where
   * program and input must share an encoding) and matches the natural
   * notebook flow: a previous ESL cell loaded the program; another
   * load brought in the input as Eigon-JSON; this call runs one
   * against the other without re-shipping bytes.
   *
   * On success, the kernel commits a trace layer and returns its
   * `traceIri`. The notebook's auto-renderer dispatches a
   * `RunProgramResponse` with a non-empty `traceIri` to a split panel
   * showing both the typed output and the program-trace tree.
   */
  async runProgramByIri(
    programIri: string,
    inputIri: string,
    options: { atLayer?: string } = {},
  ): Promise<RunProgramResponse> {
    return await this.kernel.runProgramByIri(
      create(RunProgramByIriRequestSchema, {
        programIri,
        inputIri,
        atLayer: options.atLayer ?? "",
      }),
    );
  }

  /**
   * Liveness check on the kernel. Returns kernel version, layer count,
   * resource count, and resume-sweep state.
   */
  async health(): Promise<HealthResponse> {
    return await this.kernel.health(create(HealthRequestSchema, {}));
  }

  // ------------------------------------------------------------------
  // Notebook publishing (D22 Phase 3.5)
  // ------------------------------------------------------------------

  /**
   * Translate a NotebookJson into Notebook + Cell resources and load
   * them into the active layer chain.
   *
   * The IRIs are content-addressed (see `src/notebook.ts`), so
   * publishing the same notebook twice is idempotent — the second load
   * sees the resources already in the chain and produces no new layer
   * (or an empty-delta layer, depending on backend semantics).
   *
   * Requires the notebook ontology
   * (`ontologies/notebook/notebook-ontology.json`) to be loaded first;
   * `eigen.load(notebookOntologyJson)` is idempotent the same way.
   */
  async publishNotebook(
    notebook: NotebookJson,
  ): Promise<{ publish: PublishOutput; load: LoadResponse }> {
    const publish = await notebookJsonToResources(notebook);
    const load = await this.load(JSON.stringify(publish.resources), {
      contentType: "application/eigon+json",
      autoCommit: true,
    });
    return { publish, load };
  }
}
