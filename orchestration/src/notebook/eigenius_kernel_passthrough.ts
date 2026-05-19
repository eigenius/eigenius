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
 * EigeniusKernel passthrough on the orchestrator's Connect router.
 *
 * The browser uses the same `EigeniusKernel` proto that the kernel and
 * the orchestrator's KernelClient already speak — there's no need to
 * duplicate the surface in `NotebookService`. This handler exposes a
 * curated subset of EigeniusKernel methods on the orchestrator,
 * proxying each call to the kernel via the existing KernelClient.raw
 * accessor (which preserves request/response shapes verbatim).
 *
 * Scope is intentionally minimal — only the methods the notebook MVP
 * needs, plus `health` for liveness checks. New methods are added
 * here as the notebook reaches for them; everything not registered
 * returns UNIMPLEMENTED at the Connect layer, which is the right
 * default (we don't accidentally expose kernel surface the browser
 * shouldn't reach).
 *
 * Per D22 §3.2 (browser uses existing EigeniusKernel surface for
 * methods that already exist there).
 */

import { Code, ConnectError, type ConnectRouter } from "@connectrpc/connect";
import { EigeniusKernel } from "../gen/eigenius_pb.ts";
import type { KernelClient } from "../client/kernel_client.ts";
import { operation, withRpcGuard } from "../observability/mod.ts";

export interface EigeniusKernelPassthroughDeps {
  kernel: KernelClient;
}

/**
 * Wrap a kernel passthrough call so any error thrown by the kernel
 * (typically a gRPC status from the kernel's tonic server) is
 * rethrown as a fresh `ConnectError`. Without this, the connect-node
 * grpcTransport's wrapped error leaks through the universal handler
 * with `content-type: application/grpc` and the actual message URL-
 * encoded into a `grpc-message` header — connect-web in the browser
 * can't decode that and surfaces a generic "[internal] HTTP 400".
 */
function proxy<Req, Resp>(
  op: string,
  call: (req: Req) => Promise<Resp>,
  req: Req,
): Promise<Resp> {
  return withRpcGuard(op, async (mark) => {
    try {
      return await call(req);
    } catch (err) {
      mark.fail("kernel_passthrough_failed");
      if (err instanceof ConnectError) {
        // Re-throw as a brand-new ConnectError so the universal handler
        // sees a Connect-native error and encodes it in the inbound
        // protocol's format (Connect / gRPC-Web / gRPC). We drop
        // `details` because received errors carry IncomingDetail and
        // outgoing errors expect OutgoingDetail; preserving them would
        // require schema lookups we don't have here.
        throw new ConnectError(err.rawMessage, err.code);
      }
      const message = err instanceof Error ? err.message : String(err);
      throw new ConnectError(message, Code.Internal);
    }
  });
}

export function registerEigeniusKernelPassthrough(
  router: ConnectRouter,
  deps: EigeniusKernelPassthroughDeps,
): void {
  const { kernel } = deps;

  router.service(EigeniusKernel, {
    // Read-only methods exposed in the MVP. Each is a thin call
    // through to the kernel; no orchestrator-side processing.
    inspect: (req) =>
      proxy(operation.KERNEL_PASSTHROUGH_INSPECT, kernel.raw.inspect, req),
    query: (req) =>
      proxy(operation.KERNEL_PASSTHROUGH_QUERY, kernel.raw.query, req),
    listInstitutions: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_LIST_INSTITUTIONS,
        kernel.raw.listInstitutions,
        req,
      ),
    health: (req) =>
      proxy(operation.KERNEL_PASSTHROUGH_HEALTH, kernel.raw.health, req),

    // Phase 3 (cell execution): the browser sends ESL source bytes
    // with content_type "application/x-esl" or Eigon-JSON bytes with
    // "application/eigon+json"; the kernel handles compilation as part
    // of Load. validateProgram and runProgram round-trip the same way,
    // wrapping the resource the browser already has in hand.
    load: (req) =>
      proxy(operation.KERNEL_PASSTHROUGH_LOAD, kernel.raw.load, req),
    validateProgram: (req) =>
      proxy(
        // No dedicated passthrough op constant for validate_program —
        // run_program_by_iri is the same shape (kernel passthrough),
        // so we group them under one name. Switch to a dedicated
        // constant if the dashboards want to distinguish.
        operation.KERNEL_PASSTHROUGH_RUN_PROGRAM_BY_IRI,
        kernel.raw.validateProgram,
        req,
      ),
    runProgram: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_RUN_PROGRAM_BY_IRI,
        kernel.raw.runProgram,
        req,
      ),
    runProgramByIri: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_RUN_PROGRAM_BY_IRI,
        kernel.raw.runProgramByIri,
        req,
      ),

    // Branch refs (Phase 14g). Notebook needs all four to surface a
    // branch picker, create feature branches, and prune obsolete ones.
    listBranches: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_LIST_BRANCHES,
        kernel.raw.listBranches,
        req,
      ),
    getBranch: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_GET_BRANCH,
        kernel.raw.getBranch,
        req,
      ),
    createBranch: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_CREATE_BRANCH,
        kernel.raw.createBranch,
        req,
      ),
    deleteBranch: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_DELETE_BRANCH,
        kernel.raw.deleteBranch,
        req,
      ),
    // Merge UX (D34 §6.3 / Phase 5). MergeBranches mutates a branch
    // ref through update_branch(AllowTrivial); PreviewMerge is a
    // side-effect-free LCA + IRI-disjointness walk used by the
    // Merge panel's preview pane.
    mergeBranches: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_MERGE_BRANCHES,
        kernel.raw.mergeBranches,
        req,
      ),
    previewMerge: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_PREVIEW_MERGE,
        kernel.raw.previewMerge,
        req,
      ),
    // D20 / D36 resolution surface. `prepareMerge` returns the
    // typed-conflict list the notebook's resolution flow drives;
    // `previewCascade` runs the cascade-impact dry-run; the user
    // acknowledges each cascade item and `submitResolution` commits
    // the resulting merge layer and CAS-advances the branch.
    prepareMerge: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_PREPARE_MERGE,
        kernel.raw.prepareMerge,
        req,
      ),
    previewCascade: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_PREVIEW_CASCADE,
        kernel.raw.previewCascade,
        req,
      ),
    submitResolution: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_SUBMIT_RESOLUTION,
        kernel.raw.submitResolution,
        req,
      ),
    // Compaction wizard (D34 §7 / Phase 6). Browser drives a 3-step
    // wizard: estimate → confirm → consolidate. Both RPCs round-trip
    // through to the kernel verbatim.
    consolidateChain: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_CONSOLIDATE_CHAIN,
        kernel.raw.consolidateChain,
        req,
      ),
    estimateConsolidation: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_ESTIMATE_CONSOLIDATION,
        kernel.raw.estimateConsolidation,
        req,
      ),
    // Tasks panel (D34 §9.1 / Phase 7). The notebook polls listTasks
    // for the rail destination; getTaskStatus + cancelTask back the
    // row actions.
    listTasks: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_LIST_TASKS,
        kernel.raw.listTasks,
        req,
      ),
    getTaskStatus: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_GET_TASK_STATUS,
        kernel.raw.getTaskStatus,
        req,
      ),
    cancelTask: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_CANCEL_TASK,
        kernel.raw.cancelTask,
        req,
      ),
    // Tags panel (D34 §G.2 / §8 / Phase 8). Immutable named refs;
    // CreateTag rejects duplicates with `already_exists`, DeleteTag
    // is idempotent. Tags are GC roots — created tags survive across
    // notebook sessions until explicitly deleted.
    createTag: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_CREATE_TAG,
        kernel.raw.createTag,
        req,
      ),
    listTags: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_LIST_TAGS,
        kernel.raw.listTags,
        req,
      ),
    deleteTag: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_DELETE_TAG,
        kernel.raw.deleteTag,
        req,
      ),
    // GC panel (D34 §G.4 / §9.4 / Phase 9). Two-screen flow:
    // estimate (read-only mark walk) → confirm + run (real sweep).
    // Destructive — the panel surfaces a confirmation dialog before
    // calling runGc.
    estimateGc: (req) =>
      proxy(
        operation.KERNEL_PASSTHROUGH_ESTIMATE_GC,
        kernel.raw.estimateGc,
        req,
      ),
    runGc: (req) =>
      proxy(operation.KERNEL_PASSTHROUGH_RUN_GC, kernel.raw.runGc, req),
    // Methods deferred until the relevant notebook phase needs them:
    //
    //   reflect         — not in notebook critical path
    //   getSchema       — Phase 5 (schema-aware visualisation)
    //   layerTopology   — exposed via NotebookService instead
    //
    // FIBER queries ride on the regular Query RPC under D14 (D2 v2 §3.5),
    // so a notebook FIBER cell would dispatch via Query rather than a
    // dedicated RPC.
    //
    // Add an entry here when the corresponding notebook feature is
    // ready to consume it.
  });
}
