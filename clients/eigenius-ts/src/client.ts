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
  type LayerTopologyResponse,
  LayerTopologyRequestSchema,
  NotebookService,
} from "../generated/eigenius_pb.ts";

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

export class Eigen {
  private readonly endpoint: string;
  private readonly transport: Transport;
  private readonly notebook: Client<typeof NotebookService>;

  constructor(options: EigenOptions) {
    this.endpoint = options.endpoint;
    this.transport = createConnectTransport({
      baseUrl: this.endpoint,
      fetch: options.fetch,
    });
    this.notebook = createClient(NotebookService, this.transport);
  }

  /** The orchestrator endpoint this client is bound to. */
  getEndpoint(): string {
    return this.endpoint;
  }

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
}
