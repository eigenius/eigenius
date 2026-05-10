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
 * NotebookService Connect handler — the browser-facing surface for the
 * notebook UI (D22 §3.2 / §4).
 *
 * Initially exposes one method, `LayerTopology`, which proxies to the
 * kernel's `EigeniusKernel.LayerTopology`. The browser uses this service
 * (over Connect-Web) plus the existing `EigeniusKernel` service for the
 * RPCs that already exist there (Inspect, Query, Load, RunProgram, etc.);
 * `NotebookService` exists as the place to add browser-specific RPCs as
 * new needs surface during Phases 3–5, without touching the kernel's
 * machine-to-machine surface.
 */

import type { ConnectRouter } from "@connectrpc/connect";
import {
  type LayerTopologyRequest,
  type LayerTopologyResponse,
  NotebookService,
} from "../gen/eigenius_pb.ts";
import type { KernelClient } from "../client/kernel_client.ts";
import { operation, withRpcGuard } from "../observability/mod.ts";

export interface NotebookServiceDeps {
  kernel: KernelClient;
}

/**
 * Register the NotebookService implementation on a Connect router.
 *
 * The router is the orchestrator's existing Connect router (per
 * `orchestration/src/server/mod.ts`); registering a second service on
 * it means the browser hits a single origin and the router dispatches
 * by RPC path.
 */
export function registerNotebookService(
  router: ConnectRouter,
  deps: NotebookServiceDeps,
): void {
  const { kernel } = deps;

  router.service(NotebookService, {
    layerTopology(
      req: LayerTopologyRequest,
    ): Promise<LayerTopologyResponse> {
      return withRpcGuard(operation.NOTEBOOK_LAYER_TOPOLOGY, async (mark) => {
        try {
          // Thin proxy. The kernel does the actual walking; the orchestrator
          // adds nothing to the response. Future browser-specific shaping
          // (e.g. attaching display preferences from a notebook session)
          // would happen here.
          return await kernel.layerTopology(
            req.rootLayer,
            req.maxDepth,
            req.includeResources,
          );
        } catch (e) {
          mark.fail("kernel_proxy_failed");
          throw e;
        }
      });
    },
  });
}
