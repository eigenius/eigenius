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
 * Component Handler Registry
 *
 * Manages handlers for IO components that the kernel dispatches to
 * the orchestrator. Each handler maps a component IRI to an async
 * function that executes the component logic.
 *
 * Handlers work with plain JavaScript objects. The gRPC transport
 * layer handles CBOR serialization/deserialization at the boundary.
 *
 * Architecture reference: D6 (kernel → orchestrator dispatch)
 */

/** LLM metrics recorded in a component trace. */
export interface ComponentMetrics {
  provider: string;
  model: string;
  promptTokens: number;
  completionTokens: number;
  latencyMs: number;
}

// deno-lint-ignore no-explicit-any
type EigonResource = Record<string, any>;

/** Input to a component handler. */
export interface ComponentInput {
  /** The input resource (deserialized from CBOR by the transport layer). */
  input: EigonResource;
  /** The argument resource (component configuration). */
  argument: EigonResource;
  /**
   * Auxiliary input resources for a multi-file join (D53 §4.3) — resolved by
   * the kernel from `runtime:additional_inputs` and passed to the worker as the
   * ordered tail of its input list. Empty/absent for single-input components.
   */
  additionalInputs?: EigonResource[];
}

/** Output from a component handler. */
export interface ComponentOutput {
  /** The output resource (will be serialized to CBOR by the transport layer). */
  output: EigonResource;
  /**
   * Optional raw Eigon-CBOR bytes to forward to the kernel verbatim,
   * bypassing the JS-object re-encode. Used by passthrough components
   * (e.g. RunRuntimeScript) whose worker already produced canonical
   * Eigon-CBOR: re-encoding via the decoded JS object loses
   * `data_type: json` tags (cbor-x unwraps the EIGENIUS_JSON_TAG on
   * decode, so a `canonical_proposition` term re-encodes as an untagged
   * map the kernel mis-reads as a nested resource). When set and the
   * request is CBOR, the executor sends these bytes unchanged.
   */
  outputBytes?: Uint8Array;
  /** Optional metrics (LLM token counts, latency, etc.). */
  metrics?: ComponentMetrics;
}

/** A component handler function. */
export type ComponentHandler = (
  input: ComponentInput,
) => Promise<ComponentOutput>;

/**
 * Registry of component handlers.
 *
 * Components are registered by IRI. When the kernel dispatches an IO
 * component call, the orchestrator looks up the handler here.
 */
export class ComponentRegistry {
  private handlers = new Map<string, ComponentHandler>();

  /** Register a handler for a component IRI. */
  register(componentIri: string, handler: ComponentHandler): void {
    this.handlers.set(componentIri, handler);
  }

  /** Look up a handler by component IRI. */
  get(componentIri: string): ComponentHandler | undefined {
    return this.handlers.get(componentIri);
  }

  /** Check if a handler is registered for a component IRI. */
  has(componentIri: string): boolean {
    return this.handlers.has(componentIri);
  }

  /** List all registered component IRIs. */
  listComponents(): string[] {
    return [...this.handlers.keys()];
  }

  /**
   * Execute a component by IRI.
   *
   * Looks up the handler and dispatches the call. Returns an error
   * response if no handler is registered.
   */
  async execute(
    componentIri: string,
    input: ComponentInput,
  ): Promise<ComponentOutput> {
    const handler = this.handlers.get(componentIri);
    if (!handler) {
      throw new Error(`No handler registered for component: ${componentIri}`);
    }
    return await handler(input);
  }
}
