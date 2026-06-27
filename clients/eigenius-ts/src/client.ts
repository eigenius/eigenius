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

import { type Client, createClient, type Transport } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import { create } from "@bufbuild/protobuf";
import {
  type BranchInfo,
  CancelTaskRequestSchema,
  type CancelTaskResponse,
  type ComorphismDecl,
  ConsolidateChainRequestSchema,
  type ConsolidateChainResponse,
  ConsolidateErrorKind,
  CreateBranchRequestSchema,
  type CreateBranchResponse,
  CreateTagRequestSchema,
  type CreateTagResponse,
  DeleteBranchRequestSchema,
  type DeleteBranchResponse,
  DeleteTagRequestSchema,
  type DeleteTagResponse,
  DispatchRole,
  EigeniusKernel,
  EstimateConsolidationRequestSchema,
  type EstimateConsolidationResponse,
  EstimateGcRequestSchema,
  type EstimateGcResponse,
  GetBranchRequestSchema,
  type GetBranchResponse,
  GetTaskStatusRequestSchema,
  type GetTaskStatusResponse,
  HealthRequestSchema,
  type HealthResponse,
  InspectRequestSchema,
  type InspectResponse,
  type InstitutionInfo,
  LayerTopologyRequestSchema,
  type LayerTopologyResponse,
  ListBranchesRequestSchema,
  ListInstitutionsRequestSchema,
  ListTagsRequestSchema,
  type ListTagsResponse,
  ListTasksRequestSchema,
  type ListTasksResponse,
  type CommitPolicy,
  CommitPolicy_CascadeTombstoneSchema,
  CommitPolicy_RejectSchema,
  CommitPolicySchema,
  type CommittedLayer,
  LayerRole,
  LoadRequestSchema,
  type LoadResponse,
  type CascadeAckWire,
  type CascadeItemWire,
  MergeBranchesRequestSchema,
  type MergeBranchesResponse,
  type MergeInfo,
  MergeOutcome,
  MergeQuotientKind,
  type MergeResolutionWire,
  MergeSide,
  MergeStrategyKind,
  NotebookService,
  PrepareMergeErrorKind,
  PrepareMergeRequestSchema,
  type PrepareMergeResponse,
  PreviewCascadeErrorKind,
  PreviewCascadeRequestSchema,
  type PreviewCascadeResponse,
  PreviewMergeRequestSchema,
  type PreviewMergeResponse,
  SubmitResolutionErrorKind,
  SubmitResolutionRequestSchema,
  type SubmitResolutionResponse,
  type TypedConflictWire,
  type QueryClassDecl,
  QueryRequestSchema,
  type QueryResponse,
  ReflectRequestSchema,
  type ReflectResponse,
  RunGcRequestSchema,
  type RunGcResponse,
  RunProgramByIriRequestSchema,
  RunProgramRequestSchema,
  type RunProgramResponse,
  RuntimeKind,
  type TagInfo,
  type TaskInfo,
  ValidateProgramRequestSchema,
  type ValidateProgramResponse,
  type ValidationError,
} from "../generated/eigenius_pb.ts";

// Re-export wire types so consumers don't have to reach into generated/.
export type {
  BranchInfo,
  CancelTaskResponse,
  CascadeAckWire,
  CascadeItemWire,
  CommitPolicy,
  CommittedLayer,
  ComorphismDecl,
  ConsolidateChainResponse,
  CreateBranchResponse,
  CreateTagResponse,
  DeleteBranchResponse,
  DeleteTagResponse,
  EstimateConsolidationResponse,
  EstimateGcResponse,
  GetBranchResponse,
  GetTaskStatusResponse,
  HealthResponse,
  InspectResponse,
  InstitutionInfo,
  LayerTopologyResponse,
  ListTagsResponse,
  ListTasksResponse,
  LoadResponse,
  MergeBranchesResponse,
  MergeInfo,
  MergeResolutionWire,
  PrepareMergeResponse,
  PreviewCascadeResponse,
  PreviewMergeResponse,
  QueryClassDecl,
  QueryResponse,
  ReflectResponse,
  RunGcResponse,
  RunProgramResponse,
  SubmitResolutionResponse,
  TagInfo,
  TaskInfo,
  TypedConflictWire,
  ValidateProgramResponse,
  ValidationError,
};

// Value-level enums (consumers compare against them) re-exported as
// values, not just types.
export {
  ConsolidateErrorKind,
  DispatchRole,
  LayerRole,
  MergeOutcome,
  MergeQuotientKind,
  MergeSide,
  MergeStrategyKind,
  PrepareMergeErrorKind,
  PreviewCascadeErrorKind,
  RuntimeKind,
  SubmitResolutionErrorKind,
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

  /**
   * Default branch for `load`, `runProgram`, `runProgramByIri`,
   * `inspect`, `query`, `reflect`. Empty / omitted = `"main"`. Per-call
   * `branch` options override this. Use `useBranch()` to mutate the
   * default after construction.
   */
  defaultBranch?: string;
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
   * When false (default) the walk is the lightweight stack view: only per-layer
   * summary nodes carrying per-kind counts (classes / properties / institutions /
   * instances), computed from the triple index — no resource bodies are loaded and
   * no per-resource nodes are returned, so a chain carrying a large domain lexicon
   * stays cheap. When true, additionally emits a node per resource (Class /
   * Property / Institution / instance) plus their structural edges. To inspect a
   * single layer's contents, set `rootLayer = <layer id>`, `maxDepth = 1`, and
   * `includeResources = true`.
   */
  includeResources?: boolean;
}

export interface InspectOptions {
  /**
   * Hex-encoded `LayerId` to read against. Empty / omitted = the
   * orchestrator's session active top (D21 §3.6 convention).
   * Mutually exclusive with `branch`.
   */
  atLayer?: string;

  /**
   * Branch name to read against (Phase 14g). Pin reads to this branch's
   * current head. Empty / omitted = client's `defaultBranch` (or `"main"`).
   * Mutually exclusive with `atLayer`.
   */
  branch?: string;
}

export interface QueryOptions {
  /**
   * Hex-encoded `LayerId` to evaluate the query against. Empty /
   * omitted = the orchestrator's session active top.
   * Mutually exclusive with `branch`.
   */
  atLayer?: string;

  /**
   * Branch name to read against (Phase 14g). Pin reads to this branch's
   * current head. Empty / omitted = client's `defaultBranch` (or `"main"`).
   * Mutually exclusive with `atLayer`.
   */
  branch?: string;
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

  /**
   * Branch to commit into (Phase 14g). Empty / omitted = client's
   * `defaultBranch` (or `"main"`). Branch must already exist —
   * use `createBranch()` to create one.
   */
  branch?: string;

  /**
   * Retroactive-validation policy (D41 §3.3, §8). Omit for the
   * server-side default (`{ kind: "reject", maxViolations: 100 }`).
   * `cascadeTombstone` opts into cascade-tombstoning lower-layer IRIs
   * that become invalid under the new layer's effect.
   */
  policy?: LoadPolicy;

  /**
   * IRIs to tombstone as part of this commit (D41 §10.1). Applied to
   * the initial user-layer builder before retroactive validation runs;
   * under `cascadeTombstone` they're combined with any cascade-inferred
   * tombstones.
   */
  explicitTombstones?: string[];
}

/**
 * Commit-policy shapes accepted by `Eigen.load`. Maps to the proto
 * `CommitPolicy` oneof. Absent or `{ kind: "reject" }` with no
 * `maxViolations` defers to the server-side default
 * (`Reject{ max_violations: 100 }`).
 */
export type LoadPolicy =
  | { kind: "reject"; maxViolations?: number }
  | { kind: "cascadeTombstone" };

function policyToProto(policy: LoadPolicy | undefined): CommitPolicy | undefined {
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

  /**
   * Branch the trace layer commits into (Phase 14g). Empty / omitted =
   * client's `defaultBranch` (or `"main"`).
   */
  branch?: string;
}

export interface RunProgramByIriOptions {
  /**
   * Hex-encoded `LayerId` to pin both reads to. Empty / omitted = the
   * client's `defaultBranch` head (or `"main"`).
   */
  atLayer?: string;

  /**
   * Branch the trace layer commits into (Phase 14g). Empty / omitted =
   * client's `defaultBranch` (or `"main"`). Reads still respect
   * `atLayer` independently.
   */
  branch?: string;
}

export interface CreateBranchOptions {
  /**
   * Hex-encoded `LayerId` to start the branch from. Required — branches
   * must always anchor on a known layer. Use `getBranch("main")` to
   * fetch the default starting point.
   */
  fromLayer: string;
}

export interface DeleteBranchOptions {
  /**
   * Skip the safety check that refuses to prune a branch whose head
   * matches an active task pin. Default false.
   */
  force?: boolean;
}

export interface ConsolidateOptions {
  /** Branch to consolidate. Omit (or pass empty) to default to "main". */
  branch?: string;
  /** Hex `LayerId` where the inclusive range starts (oldest). */
  fromLayer: string;
  /** Hex `LayerId` where the inclusive range ends (newest). */
  toLayer: string;
  /**
   * Cost cap. Refuse if the predicted walk size exceeds this. Pass 0
   * (the default) to use the kernel's built-in default.
   */
  maxWalkEntries?: bigint;
  /** Reserved for v2 trace-pin policies. v1 ignores this field. */
  tracePinPolicy?: string;
  /**
   * Keep the pre-consolidation history reachable for time-travel reads
   * (D25 §12.8.1(b)). Default false (reclaim mode — the source range
   * becomes GC-eligible).
   */
  preserveHistory?: boolean;
}

export class Eigen {
  private readonly endpoint: string;
  private readonly transport: Transport;
  private readonly notebook: Client<typeof NotebookService>;
  private readonly kernel: Client<typeof EigeniusKernel>;
  private defaultBranch: string;

  constructor(options: EigenOptions) {
    this.endpoint = options.endpoint;
    this.transport = createConnectTransport({
      baseUrl: this.endpoint,
      fetch: options.fetch,
    });
    this.notebook = createClient(NotebookService, this.transport);
    this.kernel = createClient(EigeniusKernel, this.transport);
    this.defaultBranch = options.defaultBranch ?? "";
  }

  /** The orchestrator endpoint this client is bound to. */
  getEndpoint(): string {
    return this.endpoint;
  }

  /**
   * Current default branch — applied when a call doesn't pass an
   * explicit `branch`. Empty string means "let the server default to
   * `main`". Returns the configured value verbatim.
   */
  getDefaultBranch(): string {
    return this.defaultBranch;
  }

  /**
   * Set the default branch for subsequent calls. Per-call `branch`
   * options still override. Pass `""` to clear and fall back to the
   * server's default (`"main"`).
   */
  useBranch(branch: string): void {
    this.defaultBranch = branch;
  }

  /**
   * Resolve a per-call branch override, falling back to the default.
   *
   * When `atLayer` is non-empty the caller has pinned the read to a
   * specific layer; the kernel rejects requests that carry **both**
   * `at_layer` and `branch`, so this returns `""` to leave the
   * mutual-exclusion contract on the wire intact. Callers that don't
   * support `at_layer` (e.g. `load`, `recordTrace`) omit the second
   * argument and the function falls through to the default-branch
   * path unchanged.
   */
  private resolveBranch(
    branch: string | undefined,
    atLayer?: string,
  ): string {
    if (atLayer !== undefined && atLayer !== "") return "";
    return branch ?? this.defaultBranch;
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
  async inspect(
    iri: string,
    options: InspectOptions = {},
  ): Promise<InspectResponse> {
    const atLayer = options.atLayer ?? "";
    return await this.kernel.inspect(
      create(InspectRequestSchema, {
        iri,
        atLayer,
        branch: this.resolveBranch(options.branch, atLayer),
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
  async query(
    eigenql: string,
    options: QueryOptions = {},
  ): Promise<QueryResponse> {
    const atLayer = options.atLayer ?? "";
    return await this.kernel.query(
      create(QueryRequestSchema, {
        eigenql,
        atLayer,
        branch: this.resolveBranch(options.branch, atLayer),
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
        branch: this.resolveBranch(options.branch),
        policy: policyToProto(options.policy),
        explicitTombstones: options.explicitTombstones ?? [],
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
        branch: this.resolveBranch(options.branch),
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
    options: RunProgramByIriOptions = {},
  ): Promise<RunProgramResponse> {
    const atLayer = options.atLayer ?? "";
    return await this.kernel.runProgramByIri(
      create(RunProgramByIriRequestSchema, {
        programIri,
        inputIri,
        atLayer,
        branch: this.resolveBranch(options.branch, atLayer),
      }),
    );
  }

  /**
   * Record a reasoning trace into a layer.
   *
   * The first resource in `trace` is treated as the trace head; the
   * server commits all parsed resources into a new layer on the named
   * branch (or the client's `defaultBranch`).
   */
  async reflect(
    trace: string | Uint8Array,
    options: { contentType?: SourceContentType; branch?: string } = {},
  ): Promise<ReflectResponse> {
    const contentType = options.contentType ?? "application/eigon+json";
    const bytes = typeof trace === "string"
      ? TEXT_ENCODER.encode(trace)
      : trace;
    return await this.kernel.reflect(
      create(ReflectRequestSchema, {
        trace: bytes,
        contentType,
        branch: this.resolveBranch(options.branch),
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
  // Branch refs (Phase 14g / D23 §5.5)
  // ------------------------------------------------------------------

  /**
   * Enumerate every branch ref the kernel currently exposes. Each
   * `BranchInfo` carries the branch name and its current head's
   * hex-encoded LayerId. Sorted by name.
   *
   * Requires a kernel with a persistent backend — the in-memory
   * variant only serves `"main"` and exposes nothing else.
   */
  async listBranches(): Promise<readonly BranchInfo[]> {
    const response = await this.kernel.listBranches(
      create(ListBranchesRequestSchema, {}),
    );
    return response.branches;
  }

  /**
   * Resolve a branch name to its current head. `found: false` means
   * the branch doesn't exist; `headLayer` is empty in that case.
   */
  async getBranch(name: string): Promise<GetBranchResponse> {
    return await this.kernel.getBranch(
      create(GetBranchRequestSchema, { name }),
    );
  }

  /**
   * Create a new branch pointing at `fromLayer`. Fails (`success: false`,
   * `error` populated) if a branch with this name already exists.
   * Server-side validates the name against `[A-Za-z0-9_-]+` (max 256
   * chars) and rejects unknown layers.
   */
  async createBranch(
    name: string,
    options: CreateBranchOptions,
  ): Promise<CreateBranchResponse> {
    return await this.kernel.createBranch(
      create(CreateBranchRequestSchema, {
        name,
        fromLayer: options.fromLayer,
      }),
    );
  }

  /**
   * Remove a branch ref. With `force: false` (default), refuses to
   * prune a branch whose head matches an active task pin. With
   * `force: true`, deletes unconditionally. Layers reachable only
   * through the deleted branch are reclaimed by the next GC pass.
   *
   * `success: true, deleted: false` means the branch didn't exist
   * (the call is idempotent).
   */
  async deleteBranch(
    name: string,
    options: DeleteBranchOptions = {},
  ): Promise<DeleteBranchResponse> {
    return await this.kernel.deleteBranch(
      create(DeleteBranchRequestSchema, {
        name,
        force: options.force ?? false,
      }),
    );
  }

  /**
   * Fold `source` into `target` (D34 §6.3). Wraps the kernel's
   * `update_branch(target, target_tip, source_tip, AllowTrivial)` —
   * succeeds as fast-forward when source is ahead of target, as
   * trivial merge when their contributions touch disjoint IRIs, or
   * surfaces `NEEDS_WITNESSED_MERGE` with the conflict set and the
   * orphan layer id when they conflict.
   */
  async mergeBranches(
    source: string,
    target: string,
  ): Promise<MergeBranchesResponse> {
    return await this.kernel.mergeBranches(
      create(MergeBranchesRequestSchema, { source, target }),
    );
  }

  /**
   * Side-effect-free preview of `mergeBranches`. Same LCA + IRI
   * disjointness walk; no merge layer built, no branch ref moved.
   * The notebook's explicit Merge dialog uses this to show
   * "Estimated outcome" before the user commits.
   */
  async previewMerge(
    source: string,
    target: string,
  ): Promise<PreviewMergeResponse> {
    return await this.kernel.previewMerge(
      create(PreviewMergeRequestSchema, { source, target }),
    );
  }

  /**
   * D36 §3.1 — Pre-compute the typed-conflict list for a (branch,
   * candidate_head) pair. Non-mutating; wraps the kernel's
   * `build_merge_span` + `classify_conflicts`. The notebook's
   * resolution flow calls this to populate the strategy picker;
   * each `TypedConflictWire` carries its kind-specific fields and
   * the strategies whose applicability check passes.
   */
  async prepareMerge(
    branch: string,
    candidateHead: string,
  ): Promise<PrepareMergeResponse> {
    return await this.kernel.prepareMerge(
      create(PrepareMergeRequestSchema, {
        branch,
        candidateHead,
      }),
    );
  }

  /**
   * D20 §7.3 — Non-mutating dry-run of cascade impact. Returns the
   * same `CascadeItemWire[]` `submitResolution` would compute,
   * without applying anything or moving any branch ref. The
   * resolution flow calls this between "user picked strategies"
   * and "user acknowledges consequences."
   */
  async previewCascade(
    branch: string,
    candidateHead: string,
    resolutions: MergeResolutionWire[],
    witnessSearchBranches: string[] = [],
  ): Promise<PreviewCascadeResponse> {
    return await this.kernel.previewCascade(
      create(PreviewCascadeRequestSchema, {
        branch,
        candidateHead,
        resolutions,
        witnessSearchBranches,
      }),
    );
  }

  /**
   * D20 §7.2 — Apply resolutions to a (branch, candidate_head) pair,
   * commit the merge layer, and CAS-advance the branch ref. The
   * caller must supply an acknowledgment for every cascade item
   * `previewCascade` would produce; missing acks surface as
   * `SubmitResolutionErrorKind::INCOMPLETE_ACKNOWLEDGMENTS`.
   */
  async submitResolution(
    branch: string,
    candidateHead: string,
    resolutions: MergeResolutionWire[],
    acknowledgments: CascadeAckWire[],
    witnessSearchBranches: string[] = [],
  ): Promise<SubmitResolutionResponse> {
    return await this.kernel.submitResolution(
      create(SubmitResolutionRequestSchema, {
        branch,
        candidateHead,
        resolutions,
        acknowledgments,
        witnessSearchBranches,
      }),
    );
  }

  /**
   * Side-effect-free dry-run of `ConsolidateChain` (D25). Same
   * validation, predicted result layer and walk cost — used by the
   * Compaction wizard's Step 2 preview before the user commits to the
   * real run.
   */
  async estimateConsolidation(
    options: ConsolidateOptions,
  ): Promise<EstimateConsolidationResponse> {
    return await this.kernel.estimateConsolidation(
      create(EstimateConsolidationRequestSchema, {
        branch: options.branch ?? "",
        fromLayer: options.fromLayer,
        toLayer: options.toLayer,
        maxWalkEntries: options.maxWalkEntries ?? 0n,
        tracePinPolicy: options.tracePinPolicy ?? "",
        preserveHistory: options.preserveHistory ?? false,
      }),
    );
  }

  /**
   * Real `ConsolidateChain` (D25 §12). Collapses the inclusive layer
   * range `[fromLayer, toLayer]` on `branch` into a single
   * consolidated layer. When the range ends at the branch tip, the
   * branch ref advances; otherwise a resolve redirect is installed at
   * `toLayer` and the branch is unchanged.
   */
  async consolidateChain(
    options: ConsolidateOptions,
  ): Promise<ConsolidateChainResponse> {
    return await this.kernel.consolidateChain(
      create(ConsolidateChainRequestSchema, {
        branch: options.branch ?? "",
        fromLayer: options.fromLayer,
        toLayer: options.toLayer,
        maxWalkEntries: options.maxWalkEntries ?? 0n,
        tracePinPolicy: options.tracePinPolicy ?? "",
        preserveHistory: options.preserveHistory ?? false,
      }),
    );
  }

  // ------------------------------------------------------------------
  // Tasks (D21)
  // ------------------------------------------------------------------

  /**
   * Snapshot of every task the kernel's TaskStore knows about — running,
   * suspended, completed, failed, cancelled. Returned in unspecified
   * order; callers sort for display.
   */
  async listTasks(): Promise<readonly TaskInfo[]> {
    const resp = await this.kernel.listTasks(
      create(ListTasksRequestSchema, {}),
    );
    return resp.tasks;
  }

  /**
   * Look up one task by id. `response.found = false` means the kernel
   * has no record for the given id (terminal tasks may be evicted from
   * the in-memory store).
   */
  async getTaskStatus(taskId: string): Promise<GetTaskStatusResponse> {
    return await this.kernel.getTaskStatus(
      create(GetTaskStatusRequestSchema, { taskId }),
    );
  }

  /**
   * Request cancellation of a running or suspended task. Idempotent on
   * already-terminal tasks: the response's `status` reflects the
   * post-cancel state (`Cancelling` / `Cancelled` if the task was
   * cancellable, otherwise the existing terminal state).
   */
  async cancelTask(taskId: string): Promise<CancelTaskResponse> {
    return await this.kernel.cancelTask(
      create(CancelTaskRequestSchema, { taskId }),
    );
  }

  // ------------------------------------------------------------------
  // Tags (D34 §G.2 / §8)
  // ------------------------------------------------------------------

  /**
   * Create a new immutable tag pointing at `layerId`. Names match
   * `[A-Za-z0-9_-]+`, max 256 chars. Rejects re-using an existing
   * name with `success: false, alreadyExists: true`; rejects an
   * unknown `layerId` with `success: false, error` populated.
   *
   * There is intentionally no `updateTag` — retargeting an existing
   * tag would defeat the "tag this state so I can come back to it
   * later" contract (D34 §8.3). Use `deleteTag` + fresh `createTag`
   * if a retarget is genuinely intended.
   */
  async createTag(
    name: string,
    layerId: string,
  ): Promise<CreateTagResponse> {
    return await this.kernel.createTag(
      create(CreateTagRequestSchema, { name, layerId }),
    );
  }

  /**
   * Enumerate every tag with its target and the target layer's
   * commit timestamp.
   */
  async listTags(): Promise<readonly TagInfo[]> {
    const resp = await this.kernel.listTags(
      create(ListTagsRequestSchema, {}),
    );
    return resp.tags;
  }

  /**
   * Remove the tag. Idempotent: deleting a non-existent tag returns
   * `success: true, deleted: false`. The target layer becomes
   * GC-eligible if no other root still reaches it.
   */
  async deleteTag(name: string): Promise<DeleteTagResponse> {
    return await this.kernel.deleteTag(
      create(DeleteTagRequestSchema, { name }),
    );
  }

  // ------------------------------------------------------------------
  // Garbage collection (D34 §G.4 / §9.4)
  // ------------------------------------------------------------------

  /**
   * Read-only dry-run of `runGc`. Same root snapshot + mark walk +
   * age classification, but performs no delete. The notebook's GC
   * panel uses this for its preview step so the operator sees what
   * a `RunGc` would actually sweep before committing.
   */
  async estimateGc(): Promise<EstimateGcResponse> {
    return await this.kernel.estimateGc(
      create(EstimateGcRequestSchema, {}),
    );
  }

  /**
   * Run a single mark-and-sweep pass. Deletes every layer that is
   * unreachable from the current root set and older than the kernel's
   * `min_age` protection window. Destructive — surface a confirmation
   * dialog in the UX before calling.
   */
  async runGc(): Promise<RunGcResponse> {
    return await this.kernel.runGc(create(RunGcRequestSchema, {}));
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
    options: { branch?: string } = {},
  ): Promise<{ publish: PublishOutput; load: LoadResponse }> {
    const publish = await notebookJsonToResources(notebook);
    const load = await this.load(JSON.stringify(publish.resources), {
      contentType: "application/eigon+json",
      autoCommit: true,
      branch: options.branch,
    });
    return { publish, load };
  }
}
