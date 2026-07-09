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
 * ComponentExecutor gRPC service implementation.
 *
 * Receives component dispatch calls from the kernel and routes them
 * to the local ComponentRegistry. This is the reverse direction:
 * kernel → orchestrator.
 *
 * Architecture reference: D6 (execution architecture).
 */

import type { ConnectRouter } from "@connectrpc/connect";
import { create } from "@bufbuild/protobuf";
import {
  ComponentExecutor,
  ComponentMetricsSchema,
  ComponentResponseSchema,
  DispatchExternalResponseSchema,
} from "../gen/eigenius_pb.ts";
import type {
  ComponentRequest,
  ComponentResponse,
  DispatchExternalRequest,
  DispatchExternalResponse,
} from "../gen/eigenius_pb.ts";
import type { ComponentRegistry } from "../components/registry.ts";
import { decodeResource, encodeResource } from "../codec/cbor.ts";
import type { RuntimeSubstrateAddon } from "../runtime/loadAddon.ts";
import * as log from "../observability/mod.ts";
import {
  type FailMark,
  operation,
  withRpcGuard,
} from "../observability/mod.ts";

const TEXT_DECODER = new TextDecoder();
const TEXT_ENCODER = new TextEncoder();

const CONTENT_TYPE_CBOR = "application/eigon+cbor";
// JSON-fallback branch is keyed on `!== CONTENT_TYPE_CBOR` (anything
// not CBOR — including the literal `application/eigon+json` and
// pre-18e clients that send empty content_type — falls through to
// JSON). No constant needed for the JSON tag.

export interface ComponentExecutorDeps {
  registry: ComponentRegistry;
  /** Optional substrate-addon handle for D31 §6.2 `DispatchExternal`
   * routing. Absent when the runtime-substrate native addon failed to
   * load — in that case `DispatchExternal` returns a typed error so
   * the kernel surfaces the misconfiguration cleanly. */
  substrate?: RuntimeSubstrateAddon;
}

/**
 * Per-request dispatcher for the ComponentExecutor service. Extracted
 * from `registerComponentExecutor` so the codec-branching logic
 * (Phase 18e: CBOR by default, JSON for backward compat) is unit-
 * testable without a real Connect server.
 *
 * The CBOR / JSON branch is symmetric: the response is encoded in the
 * same codec the request used, so a pre-18e kernel that sends JSON
 * gets JSON back during a rolling upgrade.
 */
export async function executeComponentRequest(
  req: ComponentRequest,
  registry: ComponentRegistry,
  mark: FailMark,
): Promise<ComponentResponse> {
  const componentIri = req.componentIri;

  if (!registry.has(componentIri)) {
    mark.fail("unknown_component");
    return create(ComponentResponseSchema, {
      success: false,
      error: `No handler registered for component: ${componentIri}`,
    });
  }

  try {
    // Branch on content_type per the proto field. Phase 18e:
    // kernels send Eigon-CBOR by default; the JSON path stays
    // for backward compat (mismatched kernel/orchestrator
    // versions during a rolling deploy). Empty content_type is
    // treated as JSON since pre-18e clients didn't set it.
    const useCbor = req.contentType === CONTENT_TYPE_CBOR;

    let input: Record<string, unknown>;
    let argument: Record<string, unknown>;
    if (useCbor) {
      input = decodeResource(req.input) as Record<string, unknown>;
      argument = decodeResource(req.argument) as Record<string, unknown>;
    } else {
      const inputJson = TEXT_DECODER.decode(req.input);
      const argumentJson = TEXT_DECODER.decode(req.argument);
      input = inputJson ? JSON.parse(inputJson) : {};
      argument = argumentJson ? JSON.parse(argumentJson) : {};
    }

    // Auxiliary inputs for a multi-file join (D53 §4.3) — decoded in the same
    // codec as the primary input; an empty list for ordinary single-input
    // components.
    const additionalInputs: Record<string, unknown>[] =
      (req.additionalInputs ?? [])
        .map((bytes) =>
          useCbor
            ? decodeResource(bytes) as Record<string, unknown>
            : JSON.parse(TEXT_DECODER.decode(bytes) || "{}")
        );

    const result = await registry.execute(componentIri, {
      input,
      argument,
      additionalInputs,
    });

    // Encode output in the same codec the request used so a
    // pre-18e kernel still gets JSON back during a rolling
    // upgrade.
    // Passthrough components (RunRuntimeScript) supply already-canonical
    // Eigon-CBOR via `outputBytes`; forward it verbatim so `data_type:
    // json` tags (the canonical_proposition term) survive — re-encoding
    // the decoded JS object would drop them.
    const outputBytes = useCbor && result.outputBytes
      ? result.outputBytes
      : useCbor
      ? encodeResource(result.output)
      : TEXT_ENCODER.encode(JSON.stringify(result.output));

    const response = create(ComponentResponseSchema, {
      success: true,
      output: outputBytes,
    });

    if (result.metrics) {
      response.metrics = create(ComponentMetricsSchema, {
        provider: result.metrics.provider,
        model: result.metrics.model,
        promptTokens: BigInt(result.metrics.promptTokens),
        completionTokens: BigInt(result.metrics.completionTokens),
        latencyMs: BigInt(result.metrics.latencyMs),
      });
    }

    return response;
  } catch (e) {
    mark.fail("dispatch_failed");
    return create(ComponentResponseSchema, {
      success: false,
      error: `Component execution failed: ${(e as Error).message}`,
    });
  }
}

/**
 * Per-request dispatcher for `DispatchExternal` (D31 §6.2). The
 * orchestrator routes the kernel's structured request fields into the
 * substrate addon's `dispatchExternalInstitution` entry point, which
 * synthesises an env + signature internally and dispatches through
 * the registered `LanguageRuntime`. Returns the substrate's output
 * Resource bytes plus the partial RuntimeInvocation provenance bytes
 * — the kernel folds the latter into a full RuntimeInvocation when
 * 19a.6 lands the trait-shape change.
 *
 * Extracted from `registerComponentExecutor` so callers can unit-
 * test the addon-routing logic without a real Connect server.
 */
export async function executeDispatchExternalRequest(
  req: DispatchExternalRequest,
  substrate: RuntimeSubstrateAddon | undefined,
  mark: FailMark,
): Promise<DispatchExternalResponse> {
  if (!substrate) {
    mark.fail("substrate_disabled");
    throw new Error(
      "DispatchExternal received but the orchestrator's runtime substrate " +
        "addon is not loaded (build orchestration/runtime-substrate-native)",
    );
  }
  if (!req.language) {
    mark.fail("missing_language");
    throw new Error("DispatchExternal: `language` field is required");
  }
  if (!req.envIri) {
    mark.fail("missing_env_iri");
    throw new Error("DispatchExternal: `env_iri` field is required");
  }
  if (!req.imageDigest) {
    mark.fail("missing_image_digest");
    throw new Error("DispatchExternal: `image_digest` field is required");
  }
  if (!req.methodName) {
    mark.fail("missing_method_name");
    throw new Error("DispatchExternal: `method_name` field is required");
  }
  if (!req.signatureIri) {
    mark.fail("missing_signature_iri");
    throw new Error("DispatchExternal: `signature_iri` field is required");
  }

  log.debug(operation.COMPONENT_DISPATCH, "DispatchExternal dispatching", {
    institution_iri: req.institutionIri,
    env_iri: req.envIri,
    image_digest: req.imageDigest,
    method_name: req.methodName,
    language: req.language,
    input_count: req.inputResourceCbors.length,
  });

  let outcome: { output: Uint8Array; partialInvocation: Uint8Array };
  try {
    outcome = await substrate.dispatchExternalInstitution(
      req.language,
      req.envIri,
      req.imageDigest,
      req.methodName,
      req.signatureIri,
      req.inputResourceCbors,
    );
  } catch (e) {
    mark.fail("substrate_dispatch_failed");
    log.warn(
      operation.COMPONENT_DISPATCH,
      "DispatchExternal substrate dispatch failed",
      {
        institution_iri: req.institutionIri,
        error_kind: "substrate_dispatch_failed",
        error_message: e instanceof Error ? e.message : String(e),
      },
    );
    throw e;
  }

  log.info(operation.COMPONENT_DISPATCH, "DispatchExternal completed", {
    institution_iri: req.institutionIri,
    output_bytes: outcome.output.byteLength,
    partial_invocation_bytes: outcome.partialInvocation.byteLength,
  });

  return create(DispatchExternalResponseSchema, {
    outputResourceCbor: outcome.output,
    runtimeInvocationPartialCbor: outcome.partialInvocation,
  });
}

/**
 * Register the ComponentExecutor service implementation on a Connect router.
 */
export function registerComponentExecutor(
  router: ConnectRouter,
  deps: ComponentExecutorDeps,
): void {
  const { registry, substrate } = deps;

  router.service(ComponentExecutor, {
    execute(req: ComponentRequest) {
      return withRpcGuard(
        operation.COMPONENT_DISPATCH,
        (mark) => executeComponentRequest(req, registry, mark),
      );
    },

    dispatchExternal(req: DispatchExternalRequest) {
      return withRpcGuard(
        operation.EXTERNAL_DISPATCH,
        (mark) => executeDispatchExternalRequest(req, substrate, mark),
      );
    },
  });
}
