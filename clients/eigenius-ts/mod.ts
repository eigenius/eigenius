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

// `@eigenius/client` — TypeScript SDK for the Eigenius platform.
// See D22 §5 and the `README.md` in this directory.

export {
  type BranchInfo,
  type CancelTaskResponse,
  type CascadeAckWire,
  type CascadeItemWire,
  type CommitPolicy,
  type CommittedLayer,
  type ComorphismDecl,
  type ConsolidateChainResponse,
  ConsolidateErrorKind,
  type ConsolidateOptions,
  type CreateBranchOptions,
  type CreateBranchResponse,
  type CreateTagResponse,
  type DeleteBranchOptions,
  type DeleteBranchResponse,
  type DeleteTagResponse,
  DispatchRole,
  Eigen,
  type EigenOptions,
  type EstimateConsolidationResponse,
  type EstimateGcResponse,
  type GetBranchResponse,
  type GetTaskStatusResponse,
  type HealthResponse,
  type InspectOptions,
  type InstitutionInfo,
  LayerRole,
  type LayerTopologyOptions,
  type ListTagsResponse,
  type ListTasksResponse,
  type LoadOptions,
  type LoadPolicy,
  type LoadResponse,
  type MergeBranchesResponse,
  type MergeInfo,
  MergeOutcome,
  MergeQuotientKind,
  type MergeResolutionWire,
  MergeSide,
  MergeStrategyKind,
  PrepareMergeErrorKind,
  type PrepareMergeResponse,
  PreviewCascadeErrorKind,
  type PreviewCascadeResponse,
  type PreviewMergeResponse,
  type QueryClassDecl,
  type QueryOptions,
  type QueryResponse,
  type ReflectResponse,
  type RunGcResponse,
  type RunProgramByIriOptions,
  type RunProgramOptions,
  type RunProgramResponse,
  RuntimeKind,
  type SourceContentType,
  SubmitResolutionErrorKind,
  type SubmitResolutionResponse,
  type TagInfo,
  type TaskInfo,
  type TypedConflictWire,
} from "./src/client.ts";

// Re-export the topology message + enum types so consumers don't need
// to reach into the generated/ directory.
export {
  EdgeKind,
  type LayerTopologyRequest,
  type LayerTopologyResponse,
  NodeKind,
  type TopologyEdge,
  type TopologyNode,
} from "./generated/eigenius_pb.ts";

// Notebook publishing (D22 Phase 3.5 / 4d).
export {
  type CellJson,
  type CellType,
  type EigonResource,
  type NotebookJson,
  notebookJsonToResources,
  type NotebookMetaJson,
  type ProgramRunCellJson,
  type PublishOutput,
  resourcesToNotebookJson,
  type SourceCellJson,
} from "./src/notebook.ts";
