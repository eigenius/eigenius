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
import { KernelClient } from "../client/kernel_client.ts";
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
    // Methods deferred until the relevant notebook phase needs them:
    //
    //   reflect         — not in notebook critical path
    //   fiberQuery      — Phase 3 (FIBER cell type)
    //   discoverMorphisms — out of MVP scope
    //   getSchema       — Phase 5 (schema-aware visualisation)
    //   listTasks / getTaskStatus / cancelTask — Phase 6 (task UI)
    //   layerTopology   — exposed via NotebookService instead
    //
    // Add an entry here when the corresponding notebook feature is
    // ready to consume it.
  });
}
